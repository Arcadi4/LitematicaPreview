//! The preferences page: appearance, camera feel, key bindings, preview workers, and update
//! behaviour. Every change saves immediately through `App::save_preferences`.
//!
//! The window's sidebar lists the [`PreferencesSection`]s and the main pane shows the selected
//! one, so a long controls list never becomes one unstructured scroll. A section is a stack of
//! [`group`]s of [`setting`] rows.
//!
//! Key recording uses two paths. The recorder control captures keys focus navigation or button
//! activation would take first, and `recording_key` records everything else from the window's
//! logical key event so typed characters follow the active keyboard layout.

use rune::{
    Align, Anim, Drag, El, Key, KeyEvent, LiveRegion, Mods, Phase, PhysicalInput, PhysicalKeyCode,
    Shadow, Spring, SpringPair, TextStyle, Transition, Tx, col, el, icon, row, text,
};
use rune_kit::shell::{Sidebar, SidebarRow};
use rune_kit::{Button, ButtonVariant, ControlSize, Divider, Kit, SegmentedControl, Switch};

use crate::app::App;
use crate::icons::Icon;
use crate::preferences::{
    Action, Conflict, KeyChord, MAX_FOV_DEGREES, MAX_SENSITIVITY, MIN_FOV_DEGREES, MIN_SENSITIVITY,
    Mode, PreviewSettings, Theme, key_chord, key_chord_from_physical,
};
use crate::update::{Source, State as UpdateState};

/// The widest a section's column grows, its side padding included.
const CONTENT_WIDTH: f32 = 680.0;
/// Points from a group's edge to its rows' text. Group titles and notes share it, so every
/// line of copy in a section starts at the same x.
const ROW_INSET: f32 = 14.0;
/// A single-line row's height; rows with an explanation grow past it.
const ROW_HEIGHT: f32 = 44.0;
/// A slider's track length, knob included.
const SLIDER_WIDTH: f32 = 168.0;
const SLIDER_KNOB: f32 = 16.0;

/// A focused area of the preferences page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PreferencesSection {
    #[default]
    Appearance,
    Controls,
    Performance,
    Updates,
}

impl PreferencesSection {
    const ALL: [PreferencesSection; 4] = [
        PreferencesSection::Appearance,
        PreferencesSection::Controls,
        PreferencesSection::Performance,
        PreferencesSection::Updates,
    ];

    pub(crate) fn title(self) -> &'static str {
        match self {
            PreferencesSection::Appearance => "Appearance",
            PreferencesSection::Controls => "Controls",
            PreferencesSection::Performance => "Performance",
            PreferencesSection::Updates => "Updates",
        }
    }

    fn glyph(self) -> &'static rune::Icon {
        match self {
            PreferencesSection::Appearance => Icon::Color.glyph(),
            PreferencesSection::Controls => Icon::Keyboard.glyph(),
            PreferencesSection::Performance => Icon::Gauge.glyph(),
            PreferencesSection::Updates => Icon::ArrowDownload.glyph(),
        }
    }
}

/// The section list for the window's sidebar, with the reset action at its foot.
pub fn sidebar(s: &App, k: &Kit) -> El {
    let mut sidebar = Sidebar::new()
        .label("Preferences sections")
        .inset(0.0)
        .header_gap(2.0)
        .row_gap(2.0);
    for section in PreferencesSection::ALL {
        sidebar = sidebar.child(
            SidebarRow::new(section.title())
                .icon(section.glyph())
                .height(32.0)
                .selected(s.preferences_section == section)
                .on_select(move |s: &mut App, tx: &mut Tx| {
                    if s.preferences_section != section {
                        s.preferences_section = section;
                        tx.scroll_to_instant("preferences-scroll", 0.0, 0.0);
                    }
                })
                .build(k)
                .key(format!("section-{}", section.title())),
        );
    }
    sidebar
        .footer(
            SidebarRow::new("Reset All Settings")
                .icon(Icon::Refresh.glyph())
                .height(32.0)
                .on_select(|s: &mut App, _: &mut Tx| {
                    s.preferences.reset();
                    s.preferences_section = PreferencesSection::default();
                    s.recording = None;
                    s.notice = Some("Settings reset to defaults.".to_owned());
                    s.save_preferences();
                })
                .build(k)
                .key("reset-settings"),
        )
        .build(k)
}

/// The selected section, in a scrolling column centred in the main pane. The shell supplies
/// the sidebar and the title.
pub fn view(s: &App, k: &Kit) -> El {
    let groups = match s.preferences_section {
        PreferencesSection::Appearance => appearance(s, k),
        PreferencesSection::Controls => controls(s, k),
        PreferencesSection::Performance => performance(s, k),
        PreferencesSection::Updates => updates(s, k),
    };
    k.scroll_bar(
        col()
            .key("preferences-scroll")
            .w_full()
            .h_full()
            .min_h(0.0)
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .items_center()
            .child(
                col()
                    .key(s.preferences_section.title())
                    .w_full()
                    .max_w(CONTENT_WIDTH)
                    .gap(24.0)
                    .px(28.0)
                    .pt(24.0)
                    .pb(28.0)
                    .children(groups),
            ),
    )
}

/// Rows on one outlined surface, ruled apart, under an optional title and over an optional
/// note.
fn group(k: &Kit, key: &str, title: Option<&str>, rows: Vec<El>, note: Option<El>) -> El {
    let mut surface = col()
        .key("rows")
        .w_full()
        .radius(k.radius.lg)
        .border(1.0, k.c.border_base);
    for (index, entry) in rows.into_iter().enumerate() {
        if index > 0 {
            surface = surface.child(
                el().key(format!("rule-{index}"))
                    .w_full()
                    .px(ROW_INSET)
                    .child(Divider::horizontal().build(k)),
            );
        }
        surface = surface.child(entry);
    }
    let mut group = col().key(key).role("group").w_full().gap(8.0);
    if let Some(title) = title {
        group = group.label(title).child(
            text(title)
                .key("title")
                .role("heading")
                .level(2)
                .px(ROW_INSET)
                .font(k.ty.small_medium)
                .color(k.c.text_muted),
        );
    }
    group.child(surface).children(note)
}

/// Muted copy under a group: what its rows share, or how to use them.
fn note(k: &Kit, copy: &str) -> El {
    text(copy)
        .key("note")
        .w_full()
        .px(ROW_INSET)
        .font(k.ty.caption())
        .color(k.c.text_muted)
        .wrap()
}

fn copy(k: &Kit, label: &str, description: Option<&str>) -> El {
    let mut copy = col().key("copy").gap(2.0).child(
        text(label)
            .key("label")
            .font(k.ty.ui)
            .color(k.c.text_base)
            .wrap(),
    );
    if let Some(description) = description {
        copy = copy.child(
            text(description)
                .key("description")
                .font(k.ty.caption())
                .color(k.c.text_muted)
                .wrap(),
        );
    }
    copy
}

fn setting_row(key: &str, copy: El, control: Option<El>) -> El {
    row()
        .key(format!("setting-{key}"))
        .w_full()
        .min_h(ROW_HEIGHT)
        .px(ROW_INSET)
        .py(8.0)
        .gap(24.0)
        .center_y()
        .child(copy.grow(1.0).min_w(0.0))
        .children(control)
}

/// One setting. The copy is hidden from accessibility because the control owns its name and
/// description.
fn setting(k: &Kit, key: &str, label: &str, description: Option<&str>, control: El) -> El {
    setting_row(
        key,
        copy(k, label, description).a11y_hidden(),
        Some(control),
    )
}

fn toggle_setting(
    k: &Kit,
    key: &str,
    label: &str,
    description: &str,
    checked: bool,
    disabled: bool,
    toggle: impl Fn(&mut App) + 'static,
) -> El {
    let control = Switch::new(checked)
        .label(label)
        .hide_label()
        .disabled(disabled)
        .on_toggle(toggle)
        .build(k)
        .description(description);
    setting(k, key, label, Some(description), control)
}

fn appearance(s: &App, k: &Kit) -> Vec<El> {
    let selected = match s.preferences.theme {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    };
    let theme_description =
        "Follow the system appearance, or pick a theme. The preview background stays dark.";
    let theme = SegmentedControl::new(&["System", "Light", "Dark"], Some(selected))
        .label("Theme")
        .width(Some(232.0))
        .on_pick(|s: &mut App, index, _| {
            s.preferences.set_theme(match index {
                1 => Theme::Light,
                2 => Theme::Dark,
                _ => Theme::System,
            });
            s.save_preferences();
        })
        .build(k)
        .description(theme_description)
        .focus_group();
    vec![group(
        k,
        "appearance-theme",
        None,
        vec![setting(k, "theme", "Theme", Some(theme_description), theme)],
        None,
    )]
}

fn controls(s: &App, k: &Kit) -> Vec<El> {
    vec![
        camera(s, k),
        bindings(
            s,
            k,
            Mode::Preview,
            Some(note(
                k,
                "Click a shortcut, then press any key or modifier. Escape cancels; Backspace clears the binding.",
            )),
        ),
        bindings(s, k, Mode::Spectator, None),
    ]
}

fn camera(s: &App, k: &Kit) -> El {
    let sensitivity = s.preferences.sensitivity;
    let fov = s.preferences.fov;
    group(
        k,
        "controls-camera",
        Some("Camera feel"),
        vec![
            slider_setting(
                k,
                Slider {
                    key: "sensitivity",
                    label: "Mouse sensitivity",
                    description: "How far the spectator camera turns for a given pointer movement.",
                    readout: format!("{:.0}%", sensitivity * 100.0),
                    value: sensitivity,
                    range: SliderRange {
                        min: MIN_SENSITIVITY,
                        max: MAX_SENSITIVITY,
                        step: 0.05,
                        grain: 0.01,
                        log: true,
                    },
                    disabled: false,
                    get: |s| s.preferences.sensitivity,
                    set: |s, value| s.preferences.set_sensitivity(value),
                },
            ),
            slider_setting(
                k,
                Slider {
                    key: "fov",
                    label: "Field of view",
                    description: "Vertical field of view in spectator mode.",
                    readout: format!("{fov:.0}°"),
                    value: fov,
                    range: SliderRange {
                        min: MIN_FOV_DEGREES,
                        max: MAX_FOV_DEGREES,
                        step: 1.0,
                        grain: 1.0,
                        log: false,
                    },
                    disabled: false,
                    get: |s| s.preferences.fov,
                    set: |s, value| s.preferences.set_fov(value),
                },
            ),
        ],
        None,
    )
}

fn bindings(s: &App, k: &Kit, mode: Mode, note: Option<El>) -> El {
    let title = format!("{} camera", mode.title());
    let rows = Action::all(mode)
        .iter()
        .map(|action| binding_row(s, k, *action))
        .collect();
    group(k, &format!("controls-{mode:?}"), Some(&title), rows, note)
}

fn binding_row(s: &App, k: &Kit, action: Action) -> El {
    let binding = s.preferences.binding(action);
    let recording = s.recording == Some(action);
    let keys = if recording {
        Vec::new()
    } else {
        binding.keycaps()
    };
    let value = if recording {
        "Press a key".to_owned()
    } else if keys.is_empty() {
        "Not bound".to_owned()
    } else {
        keys.join(" / ")
    };
    let control = row()
        .key(format!("record-{}", action.key()))
        .role("button")
        .label(action.title())
        .a11y_value(value)
        .description(if recording {
            "Press a key or modifier. Escape cancels; Backspace clears."
        } else {
            "Press to record a key for this action."
        })
        .focusable(true)
        .h(28.0)
        // Fixed so resting, hover, and recording values blend without a width flight.
        .w(104.0)
        .px(3.0)
        // Hangs past the row's inset by its own padding, so the keycaps line up with the other
        // rows' controls.
        .mr(-3.0)
        .justify_end()
        .center_y()
        .radius(k.radius.md)
        .hover_group()
        .hover_motion(
            Anim::from(Spring::new(0.16, 0.0)),
            Anim::from(Spring::new(0.16, 0.0)),
        )
        .focus_ring(2.0, 2.0, k.c.border_focus)
        .on_press_tx(move |s: &mut App, tx: &mut Tx| {
            if s.recording == Some(action) {
                s.recording = None;
                s.notice = None;
                tx.announce(
                    format!("{} recording cancelled.", action.title()),
                    LiveRegion::Polite,
                );
            } else {
                s.recording = Some(action);
                s.notice = None;
                tx.announce(
                    format!("Recording {}. Press a key or modifier.", action.title()),
                    LiveRegion::Polite,
                );
            }
        })
        .on_physical_input(move |s: &mut App, input, tx| record_physical_input(s, input, tx));
    let control = control.child(
        if recording {
            text("Press a key…")
                .key("record-prompt")
                .font(k.ty.ui_compact)
                .color(k.c.text_muted)
        } else {
            let value = if keys.is_empty() {
                text("Not bound")
                    .font(k.ty.ui_compact)
                    .color(k.c.text_faint)
            } else {
                row()
                    .gap(6.0)
                    .center_y()
                    .children(keys.iter().map(|key| keycap(k, key)))
            };
            row()
                .key("record-value")
                .h(22.0)
                .center_y()
                .child(value.group_opacity(1.0, 0.0))
                .child(
                    row()
                        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                        .justify_end()
                        .center_y()
                        .child(
                            text("Edit")
                                .key("edit-hint")
                                .font(k.ty.ui_compact)
                                .color(k.c.text_muted)
                                .group_opacity(0.0, 1.0),
                        ),
                )
        }
        .transition(Transition::opacity().springs(SpringPair::new(
            Spring::new(0.12, 0.0),
            Spring::new(0.08, 0.0),
        )))
        .enter_delay(0.09)
        .a11y_hidden(),
    );
    setting(k, action.key(), action.title(), None, control)
}

/// [`rune_kit::Keybind`]'s keycap at row scale: here the keys are a row's value, not a hint
/// beside a command.
fn keycap(k: &Kit, label: &str) -> El {
    row()
        .min_w(22.0)
        .h(22.0)
        .px(6.0)
        .center()
        .radius(k.radius.sm)
        .fill(k.c.layer_03)
        .child(
            text(label)
                .font(TextStyle {
                    size: 12.0,
                    line_height: 14.0,
                    ..k.ty.small_medium
                })
                .color(k.c.text_base),
        )
}

/// Keys focus navigation or button activation would take before the app's key handler, plus
/// modifiers. Typed characters are left to [`recording_key`] so the active layout decides them.
fn needs_physical_capture(key: PhysicalKeyCode) -> bool {
    use PhysicalKeyCode::*;
    matches!(
        key,
        ControlLeft
            | ControlRight
            | ShiftLeft
            | ShiftRight
            | AltLeft
            | AltRight
            | SuperLeft
            | SuperRight
            | Fn
            | FnLock
            | CapsLock
            | Space
            | Enter
            | NumpadEnter
            | Tab
            | Escape
            | Backspace
            | ArrowLeft
            | ArrowRight
            | ArrowUp
            | ArrowDown
            | Home
            | End
            | PageUp
            | PageDown
    )
}

/// Records a key the settings page receives before focus navigation. Returns true for the keys
/// it captures, so the viewer never sees a key meant for the recorder.
pub fn record_physical_input(s: &mut App, input: &PhysicalInput, tx: &mut Tx) -> bool {
    let Some(action) = s.recording else {
        return false;
    };
    let PhysicalInput::Key {
        key,
        mods,
        pressed,
        repeat,
    } = *input
    else {
        if matches!(input, PhysicalInput::FocusLost) {
            s.recording = None;
            s.notice = Some("Key recording cancelled.".to_owned());
            tx.announce("Key recording cancelled.", LiveRegion::Polite);
        }
        return true;
    };
    if !pressed || repeat {
        return true;
    }
    if key == PhysicalKeyCode::Escape && mods == Mods::default() {
        cancel(s, action, tx);
        return true;
    }
    if key == PhysicalKeyCode::Backspace && mods == Mods::default() {
        clear(s, action, tx);
        return true;
    }
    if !needs_physical_capture(key) {
        return false;
    }
    let Some(key) = key_chord_from_physical(key, mods) else {
        return true;
    };
    assign(s, action, key, tx);
    true
}

/// Records a typed key from the window's key handler, using its layout-dependent character.
/// Physical capture handles the keys focus navigation takes before this handler runs.
pub fn recording_key(s: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(action) = s.recording else {
        return false;
    };
    if matches!(&event.key, Key::Escape) && event.mods == Mods::default() {
        cancel(s, action, tx);
        return true;
    }
    if matches!(&event.key, Key::Backspace) && event.mods == Mods::default() {
        clear(s, action, tx);
        return true;
    }
    if tx.key_repeat() {
        return true;
    }
    if let Some(key) = key_chord(event) {
        assign(s, action, key, tx);
    }
    true
}

fn assign(s: &mut App, action: Action, key: KeyChord, tx: &mut Tx) {
    match s.preferences.assign_binding(action, key) {
        Ok(()) => {
            let message = format!(
                "{} set to {}.",
                action.title(),
                s.preferences.binding(action).label()
            );
            s.recording = None;
            s.notice = Some(message.clone());
            s.save_preferences();
            tx.announce(message, LiveRegion::Polite);
        }
        Err(Conflict { action: used }) => {
            let message = format!("{} is already used by {}.", key.label(), used.title());
            s.notice = Some(message.clone());
            tx.announce(message, LiveRegion::Assertive);
        }
    }
}

fn cancel(s: &mut App, action: Action, tx: &mut Tx) {
    s.recording = None;
    s.notice = None;
    tx.announce(
        format!("{} recording cancelled.", action.title()),
        LiveRegion::Polite,
    );
}

fn clear(s: &mut App, action: Action, tx: &mut Tx) {
    s.preferences.clear_binding(action);
    s.recording = None;
    let message = format!("{} cleared.", action.title());
    s.notice = Some(message.clone());
    s.save_preferences();
    tx.announce(message, LiveRegion::Polite);
}

fn performance(s: &App, k: &Kit) -> Vec<El> {
    let preview = s.preferences.preview;
    let chunking_description = if preview.multithreading_enabled {
        "Smaller chunks reduce peak meshing memory and allow cancellation between chunks. Turn off multithreading to change this."
    } else {
        "Smaller chunks reduce peak meshing memory and allow cancellation between chunks."
    };
    let chunking = toggle_setting(
        k,
        "chunking",
        "Separate geometry into chunks",
        chunking_description,
        preview.chunking_enabled,
        preview.multithreading_enabled,
        |s: &mut App| {
            let enabled = !s.preferences.preview.chunking_enabled;
            s.preferences.set_chunking(enabled);
            s.save_preferences();
        },
    );
    let chunk_size = slider_setting(
        k,
        Slider {
            key: "chunk-size",
            label: "Chunk size",
            description: "The edge length, in blocks, of each geometry chunk.",
            readout: preview.chunk_size.to_string(),
            value: chunk_size_index(preview.chunk_size),
            range: SliderRange {
                min: 0.0,
                max: (PreviewSettings::CHUNK_SIZES.len() - 1) as f32,
                step: 1.0,
                grain: 1.0,
                log: false,
            },
            disabled: !preview.chunking_enabled,
            get: |s| chunk_size_index(s.preferences.preview.chunk_size),
            set: |s, index| {
                if let Some(size) = PreviewSettings::CHUNK_SIZES.get(index as usize) {
                    s.preferences.set_chunk_size(*size);
                }
            },
        },
    );
    let multithreading_description = if preview.chunking_enabled {
        "Uses more CPU cores while preparing and meshing a schematic."
    } else {
        "Uses more CPU cores while preparing and meshing a schematic. Turn on chunking to use multithreading."
    };
    let multithreading = toggle_setting(
        k,
        "multithreading",
        "Enable multithreading",
        multithreading_description,
        preview.multithreading_enabled,
        !preview.chunking_enabled,
        |s: &mut App| {
            let enabled = !s.preferences.preview.multithreading_enabled;
            s.preferences.set_multithreading(enabled);
            s.save_preferences();
        },
    );
    let threaded = preview.chunking_enabled && preview.multithreading_enabled;
    let thread_count = slider_setting(
        k,
        Slider {
            key: "threads",
            label: "Worker threads",
            description: if threaded {
                "The number of worker threads used while meshing."
            } else {
                "The number of worker threads used while meshing. Unavailable while chunking or multithreading is off."
            },
            readout: preview.thread_count.to_string(),
            value: f32::from(preview.thread_count),
            range: SliderRange {
                min: f32::from(PreviewSettings::MIN_THREADS),
                max: f32::from(PreviewSettings::MAX_THREADS),
                step: 1.0,
                grain: 1.0,
                log: false,
            },
            disabled: !threaded,
            get: |s| f32::from(s.preferences.preview.thread_count),
            set: |s, count| s.preferences.set_thread_count(count as u8),
        },
    );
    let footer = row()
        .key("preview-footer")
        .w_full()
        .gap(16.0)
        .center_y()
        .child(
            note(k, "Changes apply the next time you open a schematic.")
                .grow(1.0)
                .min_w(0.0),
        )
        .child(
            Button::new("Reset preview settings")
                .icon(Icon::Refresh.glyph())
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| {
                    s.preferences.preview = PreviewSettings::default();
                    s.notice = Some("Preview settings reset.".to_owned());
                    s.save_preferences();
                })
                .build(k)
                .key("reset-preview-settings"),
        );
    vec![group(
        k,
        "preview",
        Some("Preview preparation"),
        vec![chunking, chunk_size, multithreading, thread_count],
        Some(footer),
    )]
}

fn chunk_size_index(size: u32) -> f32 {
    PreviewSettings::CHUNK_SIZES
        .iter()
        .position(|chunk| *chunk == size)
        .unwrap_or_default() as f32
}

const APPIMAGE_UPDATE_URL: &str = "https://github.com/AppImageCommunity/AppImageUpdate/issues";

fn updates(s: &App, k: &Kit) -> Vec<El> {
    let mut rows = Vec::new();
    match &s.update_mode {
        crate::update::Mode::SelfUpdate => {
            rows.push(toggle_setting(
                k,
                "auto-update",
                "Check for updates automatically",
                "When off, no update check runs at launch and the download source is hidden.",
                s.preferences.auto_update,
                false,
                |s: &mut App| {
                    let enabled = !s.preferences.auto_update;
                    s.preferences.set_auto_update(enabled);
                    s.save_preferences();
                },
            ));
            if s.preferences.auto_update {
                let selected = match &s.preferences.update_source {
                    Source::Cloudflare => 1,
                    _ => 0,
                };
                let source_description = "Where update checks and downloads come from.";
                let source = SegmentedControl::new(&["GitHub", "Cloudflare"], Some(selected))
                    .label("Update source")
                    .width(Some(232.0))
                    .on_pick(|s: &mut App, index, _| {
                        s.preferences.set_update_source(if index == 1 {
                            Source::Cloudflare
                        } else {
                            Source::Github
                        });
                        s.save_preferences();
                    })
                    .build(k)
                    .description(source_description)
                    .focus_group();
                rows.push(setting(
                    k,
                    "source",
                    "Update source",
                    Some(source_description),
                    source,
                ));
                let in_flight = matches!(
                    &s.update_state,
                    UpdateState::Checking | UpdateState::Installing
                );
                let mut actions = row().gap(8.0).center_y().child(
                    Button::new("Check for updates")
                        .icon(Icon::Refresh.glyph())
                        .disabled(in_flight)
                        .on_press(|s: &mut App| s.check_update())
                        .build(k)
                        .key("check-updates"),
                );
                if matches!(&s.update_state, UpdateState::Available(_)) {
                    actions = actions.child(
                        Button::new("Install update")
                            .icon(Icon::ArrowDownload.glyph())
                            .variant(ButtonVariant::Contrast)
                            .on_press(|s: &mut App| s.install_update())
                            .build(k)
                            .key("install-update"),
                    );
                }
                rows.push(version_row(s, k, None, Some(actions)));
            }
        }
        crate::update::Mode::AppImage => rows.push(version_row(
            s,
            k,
            Some(
                "This build is an AppImage. Updates are installed with AppImageUpdate; the download source is not configurable here.",
            ),
            Some(
                Button::new("Check with AppImageUpdate")
                    .icon(Icon::ArrowDownload.glyph())
                    .on_press_tx(|_: &mut App, tx: &mut Tx| {
                        let _ = tx.open_url(APPIMAGE_UPDATE_URL);
                    })
                    .build(k)
                    .key("check-appimage-update"),
            ),
        )),
        _ => rows.push(version_row(
            s,
            k,
            Some(
                "Updates are managed by your package manager. Update actions and the download source are unavailable in this build.",
            ),
            None,
        )),
    }
    vec![group(k, "updates-card", None, rows, None)]
}

/// The running version, the latest check's outcome and `description` under it, and the update
/// actions this build offers beside it.
fn version_row(s: &App, k: &Kit, description: Option<&str>, actions: Option<El>) -> El {
    let mut copy = copy(
        k,
        &format!("Litematica Preview {}", crate::update::current_version()),
        description,
    );
    if let Some(status) = update_status(s, k) {
        copy = copy.child(status);
    }
    setting_row("version", copy, actions)
}

fn update_status(s: &App, k: &Kit) -> Option<El> {
    let (message, color, glyph) = match &s.update_state {
        UpdateState::Idle => return None,
        UpdateState::Checking => (
            "Checking for updates…".to_owned(),
            k.c.text_muted,
            Icon::Info.glyph(),
        ),
        UpdateState::Current => (
            "Litematica Preview is up to date.".to_owned(),
            k.c.success.fg,
            Icon::Checkmark.glyph(),
        ),
        UpdateState::Available(release) => (
            format!("Version {} is available.", release.version),
            k.c.text_base,
            Icon::ArrowDownload.glyph(),
        ),
        UpdateState::Installing => (
            "Installing the update…".to_owned(),
            k.c.text_muted,
            Icon::Info.glyph(),
        ),
        UpdateState::Installed => (
            "Update installed. Restart Litematica Preview to finish.".to_owned(),
            k.c.success.fg,
            Icon::Checkmark.glyph(),
        ),
        UpdateState::Failed(error) => (
            format!("The update could not be completed: {error}"),
            k.c.danger.fg,
            Icon::Dismiss.glyph(),
        ),
    };
    Some(
        row()
            .key("update-status")
            .role("status")
            .w_full()
            .gap(4.0)
            .pt(2.0)
            .center_y()
            .child(icon(glyph, 12.0).color(color).a11y_hidden())
            .child(
                text(message)
                    .font(k.ty.caption())
                    .color(color)
                    .wrap()
                    .grow(1.0),
            ),
    )
}

/// How a slider's value spans its track.
#[derive(Clone, Copy)]
struct SliderRange {
    min: f32,
    max: f32,
    /// What one arrow key moves the value by.
    step: f32,
    /// What every set value rounds to.
    grain: f32,
    /// Spaces the track geometrically, so every doubling takes the same travel.
    log: bool,
}

impl SliderRange {
    fn position(self, value: f32) -> f32 {
        let position = if self.log {
            (value / self.min).ln() / (self.max / self.min).ln()
        } else {
            (value - self.min) / (self.max - self.min)
        };
        position.clamp(0.0, 1.0)
    }

    fn value(self, position: f32) -> f32 {
        let position = position.clamp(0.0, 1.0);
        self.snap(if self.log {
            self.min * (self.max / self.min).powf(position)
        } else {
            self.min + position * (self.max - self.min)
        })
    }

    fn snap(self, value: f32) -> f32 {
        ((value / self.grain).round() * self.grain).clamp(self.min, self.max)
    }
}

/// A slider row: what it shows, and how it reads and writes the preference behind it.
struct Slider<'a> {
    key: &'a str,
    label: &'a str,
    description: &'a str,
    readout: String,
    value: f32,
    range: SliderRange,
    disabled: bool,
    get: fn(&App) -> f32,
    set: fn(&mut App, f32),
}

/// A knob on a track with the value read out beside it. A press on the track jumps the knob
/// there; a drag that starts on the knob keeps the knob where it was grabbed. Arrow keys step,
/// Home and End go to the ends. The value saves after each step and at the end of a drag.
fn slider_setting(k: &Kit, slider: Slider) -> El {
    let Slider {
        key,
        label,
        description,
        readout,
        value,
        range,
        disabled,
        get,
        set,
    } = slider;
    let height = 20.0;
    let rail = 4.0;
    let travel = SLIDER_WIDTH - SLIDER_KNOB;
    let knob_at = move |value: f32| SLIDER_KNOB / 2.0 + range.position(value) * travel;
    let on_knob = move |x: f32, value: f32| (x - knob_at(value)).abs() <= SLIDER_KNOB / 2.0 + 2.0;
    let value_at = move |x: f32| range.value((x - SLIDER_KNOB / 2.0) / travel);
    let position = range.position(value);
    let knob = el()
        .key("knob")
        .size(SLIDER_KNOB, SLIDER_KNOB)
        .absolute(position * travel, (height - SLIDER_KNOB) / 2.0)
        .radius(SLIDER_KNOB / 2.0)
        .fill(k.c.knob)
        .box_shadow(Shadow {
            spread: 0.5,
            color: k.c.border_strong,
            ..Default::default()
        })
        .motion(k.motion.control)
        .a11y_hidden();
    let mut track = el()
        .key(format!("slider-{key}"))
        .role("slider")
        .label(label)
        .description(description)
        .range_value(f64::from(value), f64::from(range.min), f64::from(range.max))
        .range_step(f64::from(range.step))
        .a11y_value(readout.clone())
        .disabled(disabled)
        .size(SLIDER_WIDTH, height)
        .radius(height / 2.0)
        .child(
            el().key("rail")
                .size(SLIDER_WIDTH, rail)
                .absolute(0.0, (height - rail) / 2.0)
                .radius(rail / 2.0)
                .fill(k.c.layer_03)
                .a11y_hidden(),
        )
        .child(
            el().key("fill")
                .size(SLIDER_KNOB / 2.0 + position * travel, rail)
                .absolute(0.0, (height - rail) / 2.0)
                .radius(rail / 2.0)
                .fill(k.c.bg_accent)
                .motion(k.motion.control)
                .a11y_hidden(),
        )
        .child(k.elevate(knob, &k.elevation.knob));
    track = if disabled {
        track.layer_opacity(0.5)
    } else {
        track
            .focus_ring(2.0, 2.0, k.c.border_focus)
            .on_step(move |s: &mut App, steps, _| {
                set(s, range.snap(get(s) + steps as f32 * range.step));
                s.save_preferences();
            })
            .on_step_limit(move |s: &mut App, end, _| {
                set(s, if end { range.max } else { range.min });
                s.save_preferences();
            })
            .on_drag(
                move |s: &mut App, d: &mut Drag, tx: &mut Tx| match d.phase {
                    Phase::Down => {
                        d.data = get(s);
                        if !on_knob(d.x, d.data) {
                            set(s, value_at(d.x));
                        }
                    }
                    Phase::Move => {
                        let grab = if on_knob(d.start_x, d.data) {
                            d.start_x - knob_at(d.data)
                        } else {
                            0.0
                        };
                        set(s, value_at(d.x - grab));
                        tx.instant();
                    }
                    Phase::Up => {
                        if d.cancelled() {
                            set(s, d.data);
                        }
                        s.save_preferences();
                    }
                },
            )
    };
    let control = row()
        .key(format!("slider-row-{key}"))
        .gap(12.0)
        .center_y()
        .child(track)
        .child(
            text(readout)
                .key("readout")
                .w(44.0)
                .align(Align::End)
                .font(TextStyle {
                    features: "'tnum' 1",
                    ..k.ty.ui
                })
                .color(if disabled {
                    k.c.text_faint
                } else {
                    k.c.text_muted
                })
                .a11y_hidden(),
        );
    setting(k, key, label, Some(description), control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::{HoldKey, Preferences};

    fn test_app() -> App {
        let mut app = App::new();
        app.preferences = Preferences::default();
        app.preferences_path = std::env::temp_dir().join(format!(
            "litematica-preview-recording-{}.json",
            std::process::id()
        ));
        app
    }

    fn key_event(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            mods: Mods::default(),
        }
    }

    #[test]
    fn a_typed_key_records_its_layout_character_not_the_physical_qwerty_one() {
        let mut app = test_app();
        app.recording = Some(Action::FlyForward);
        let mut tx = Tx::default();
        let w = PhysicalInput::Key {
            key: PhysicalKeyCode::KeyW,
            mods: Mods::default(),
            pressed: true,
            repeat: false,
        };
        assert!(!record_physical_input(&mut app, &w, &mut tx));
        assert_eq!(app.recording, Some(Action::FlyForward));

        // Colemak types `,` where QWERTY types `w`.
        let colemak_w = key_event(Key::Text(",".into()));
        assert!(recording_key(&mut app, &colemak_w, &mut tx));

        let binding = app.preferences.binding(Action::FlyForward);
        assert!(binding.matches(&colemak_w, &tx));
        assert!(!binding.matches(&key_event(Key::Text("w".into())), &tx));
    }

    #[test]
    fn focus_keys_and_modifiers_still_record_from_physical_input() {
        let mut app = test_app();
        let mut tx = Tx::default();
        app.recording = Some(Action::Fit);
        let tab = PhysicalInput::Key {
            key: PhysicalKeyCode::Tab,
            mods: Mods::default(),
            pressed: true,
            repeat: false,
        };
        assert!(record_physical_input(&mut app, &tab, &mut tx));
        assert_eq!(app.recording, None);
        assert!(
            app.preferences
                .binding(Action::Fit)
                .matches(&key_event(Key::Tab), &tx)
        );

        app.recording = Some(Action::FlyForward);
        let shift = PhysicalInput::Key {
            key: PhysicalKeyCode::ShiftLeft,
            mods: Mods::default(),
            pressed: true,
            repeat: false,
        };
        assert!(record_physical_input(&mut app, &shift, &mut tx));
        assert_eq!(app.recording, None);
        assert_eq!(
            app.preferences.binding(Action::FlyForward).label(),
            KeyChord::Hold(HoldKey::Shift).label()
        );
    }

    #[test]
    fn a_focused_recorder_records_the_logical_key_the_runtime_delivers() {
        use rune::{Env, Runtime};

        use crate::app::Page;

        let mut app = test_app();
        app.page = Page::Preferences;
        app.preferences_section = PreferencesSection::Controls;
        app.recording = Some(Action::FlyForward);
        let mut runtime =
            Runtime::new(app, crate::view::view, (1120.0, 740.0), Env::new(true, 1.0));
        runtime.on_key = Some(App::key);
        runtime.frame(0.0);
        let (x, y) = runtime
            .find("record-flyForward")
            .expect("recorder is in view");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);

        runtime.physical_input(PhysicalInput::Key {
            key: PhysicalKeyCode::KeyW,
            mods: Mods::default(),
            pressed: true,
            repeat: false,
        });
        runtime.key(key_event(Key::Text(",".into())));
        runtime.frame(0.0);

        assert_eq!(runtime.state.recording, None);
        let colemak_w = key_event(Key::Text(",".into()));
        assert!(
            runtime
                .state
                .preferences
                .binding(Action::FlyForward)
                .matches(&colemak_w, &Tx::default())
        );
    }
}
