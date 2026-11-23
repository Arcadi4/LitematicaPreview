//! The preferences page: appearance, camera feel, key bindings, preview workers, and update
//! behaviour. Every change saves immediately through `App::save_preferences`.
//!
//! The window's sidebar lists the [`PreferencesSection`]s and the main pane shows the selected
//! one, so a long controls list never becomes one unstructured scroll. A section is a stack of
//! [`group`]s of [`setting`] rows.
//!
//! Key recording uses two paths so it works with every key. The recorder control receives
//! physical input before focus navigation (so Space, Return and modifiers can be recorded),
//! and `recording_key` handles the same job from a window-level key handler. Both write through
//! the same preference API, so a recording cannot change the viewer while it runs.

use rune::{
    El, Key, KeyEvent, LiveRegion, Mods, PhysicalInput, PhysicalKeyCode, Tx, col, el, icon, row,
    text,
};
use rune_kit::shell::{Sidebar, SidebarRow};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Divider, Keybind, Kit, SegmentedControl, State, Switch,
};

use crate::app::App;
use crate::icons::Icon;
use crate::preferences::{
    Action, Conflict, KeyChord, Mode, PreviewSettings, Theme, key_chord, key_chord_from_physical,
};
use crate::update::{Source, State as UpdateState};

/// The widest a section's column grows, its side padding included.
const CONTENT_WIDTH: f32 = 680.0;
/// Points from a group's edge to its rows' text. Group titles and notes share it, so every
/// line of copy in a section starts at the same x.
const ROW_INSET: f32 = 14.0;
/// A single-line row's height; rows with an explanation grow past it.
const ROW_HEIGHT: f32 = 44.0;

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
    let sensitivity_value = format!("{:.0}%", sensitivity * 100.0);
    let fov_value = format!("{fov:.0}°");
    group(
        k,
        "controls-camera",
        Some("Camera feel"),
        vec![
            stepper_setting(
                k,
                "sensitivity",
                "Mouse sensitivity",
                "How far the spectator camera turns for a given pointer movement.",
                &sensitivity_value,
                "Lower mouse sensitivity",
                "Raise mouse sensitivity",
                false,
                |s| {
                    s.preferences
                        .set_sensitivity(s.preferences.sensitivity - 0.05)
                },
                |s| {
                    s.preferences
                        .set_sensitivity(s.preferences.sensitivity + 0.05)
                },
            ),
            stepper_setting(
                k,
                "fov",
                "Field of view",
                "Vertical field of view in spectator mode.",
                &fov_value,
                "Narrow the field of view",
                "Widen the field of view",
                false,
                |s| s.preferences.set_fov(s.preferences.fov - 1.0),
                |s| s.preferences.set_fov(s.preferences.fov + 1.0),
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
    let mut control = row()
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
        .h(26.0)
        .min_w(64.0)
        .px(6.0)
        // Hangs past the row's inset by its own padding, so the keycaps line up with the other
        // rows' controls.
        .mr(-6.0)
        .justify_end()
        .center_y()
        .radius(k.radius.md)
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
    control = if recording {
        control.fill(k.c.layer_01).border(1.0, k.c.border_focus)
    } else {
        k.interactive(control)
    };
    control = control.child(
        if recording {
            text("Press a key…")
                .font(k.ty.ui_compact)
                .color(k.c.text_muted)
        } else if keys.is_empty() {
            text("Not bound")
                .font(k.ty.ui_compact)
                .color(k.c.text_faint)
        } else {
            row()
                .gap(6.0)
                .center_y()
                .children(keys.iter().map(|key| Keybind::new(&[key]).build(k)))
        }
        .a11y_hidden(),
    );
    setting(k, action.key(), action.title(), None, control)
}

/// Records a key the settings page receives before focus navigation. Returns true while
/// recording, so the viewer never sees a key meant for the recorder.
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
    let Some(key) = key_chord_from_physical(key, mods) else {
        return true;
    };
    assign(s, action, key, tx);
    true
}

/// Records a key from the window's key handler. This is the fallback for a recorder that does
/// not hold focus; the settings page's recorder usually takes the physical path first.
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
    let sizes: Vec<String> = PreviewSettings::CHUNK_SIZES
        .iter()
        .map(|size| size.to_string())
        .collect();
    let selected = PreviewSettings::CHUNK_SIZES
        .iter()
        .position(|size| *size == preview.chunk_size);
    let mut chunk_size = SegmentedControl::new(&sizes, selected)
        .fit()
        .label("Chunk size")
        .on_pick(|s: &mut App, index, _| {
            if let Some(size) = PreviewSettings::CHUNK_SIZES.get(index) {
                s.preferences.set_chunk_size(*size);
                s.save_preferences();
            }
        });
    if !preview.chunking_enabled {
        for index in 0..sizes.len() {
            chunk_size = chunk_size.segment_state(index, State::Disabled);
        }
    }
    let chunk_size_description = "The edge length, in blocks, of each geometry chunk.";
    let chunk_size = setting(
        k,
        "chunk-size",
        "Chunk size",
        Some(chunk_size_description),
        chunk_size
            .build(k)
            .description(chunk_size_description)
            .focus_group(),
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
    let thread_count = stepper_setting(
        k,
        "threads",
        "Worker threads",
        "The number of worker threads used while meshing.",
        &preview.thread_count.to_string(),
        "Use fewer worker threads",
        "Use more worker threads",
        !threaded,
        |s| {
            s.preferences
                .set_thread_count(s.preferences.preview.thread_count.saturating_sub(1));
        },
        |s| {
            s.preferences
                .set_thread_count(s.preferences.preview.thread_count.saturating_add(1));
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

#[allow(clippy::too_many_arguments)]
fn stepper_setting(
    k: &Kit,
    key: &str,
    label: &str,
    description: &str,
    value: &str,
    decrease: &str,
    increase: &str,
    disabled: bool,
    lower: impl Fn(&mut App) + 'static,
    higher: impl Fn(&mut App) + 'static,
) -> El {
    let down = Icon::ChevronDown.glyph();
    let up = down;
    let minus = Button::new(decrease)
        .label_with(move |_, ink| icon(down, 12.0).color(ink))
        .control_size(ControlSize::Mini)
        .help(decrease)
        .disabled(disabled)
        .on_press(move |s: &mut App| {
            lower(s);
            s.save_preferences();
        })
        .build(k);
    let plus = Button::new(increase)
        .label_with(move |_, ink| icon(up, 12.0).color(ink).rotate(std::f32::consts::PI))
        .control_size(ControlSize::Mini)
        .help(increase)
        .disabled(disabled)
        .on_press(move |s: &mut App| {
            higher(s);
            s.save_preferences();
        })
        .build(k);
    let mut description = description.to_owned();
    if disabled {
        description.push_str(" Unavailable while chunking or multithreading is off.");
    }
    let control = row()
        .key(format!("stepper-{key}"))
        .gap(8.0)
        .center_y()
        .role("group")
        .label(label)
        .a11y_value(value)
        .description(description.clone())
        .child(minus)
        .child(
            text(value)
                .key("value")
                .min_w(52.0)
                .center()
                .font(k.ty.ui_medium)
                .color(k.c.text_base),
        )
        .child(plus);
    let control = if disabled {
        control.layer_opacity(0.5)
    } else {
        control
    };
    setting(k, key, label, Some(&description), control)
}
