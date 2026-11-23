//! The preferences page: appearance, camera feel, key bindings, preview workers, and update
//! behaviour. Every change saves immediately through `App::save_preferences`.
//!
//! The page is split into focused sections behind a kit segmented nav, so a long controls list
//! never becomes one unstructured scroll. Each group is a [`Card`] of kit rows; controls use the
//! packaged switch, segmented, button, keybind and divider widgets.
//!
//! Key recording uses two paths so it works with every key. The recorder control receives
//! physical input before focus navigation (so Space, Return and modifiers can be recorded),
//! and `recording_key` handles the same job from a window-level key handler. Both write through
//! the same preference API, so a recording cannot change the viewer while it runs.

use rune::{
    El, Key, KeyEvent, LiveRegion, Mods, PhysicalInput, PhysicalKeyCode, Tx, col, el, icon, row,
    text,
};
use rune_kit::{
    Button, ButtonVariant, Card, ControlSize, Divider, Keybind, Kit, SegmentedControl, State,
    Switch,
};

use crate::icons::Icon;
use crate::preferences::{
    Action, Conflict, KeyChord, Mode, PreviewSettings, Theme, key_chord, key_chord_from_physical,
};
use crate::update::{Source, State as UpdateState};

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

    const LABELS: [&'static str; 4] = ["Appearance", "Controls", "Performance", "Updates"];

    fn index(self) -> usize {
        match self {
            PreferencesSection::Appearance => 0,
            PreferencesSection::Controls => 1,
            PreferencesSection::Performance => 2,
            PreferencesSection::Updates => 3,
        }
    }

    fn from_index(index: usize) -> PreferencesSection {
        Self::ALL.get(index).copied().unwrap_or_default()
    }
}

/// Draws the body of the preferences page. The shell supplies the page frame and title.
pub fn view(s: &crate::app::App, kit: &Kit) -> El {
    let k = kit;
    let nav = SegmentedControl::new(
        &PreferencesSection::LABELS,
        Some(s.preferences_section.index()),
    )
    .label("Preferences sections")
    .fit()
    .on_pick(|s: &mut crate::app::App, index, tx| {
        s.preferences_section = PreferencesSection::from_index(index);
        tx.scroll_to_instant("preferences-scroll", 0.0, 0.0);
    })
    .build(k)
    .key("preferences-nav")
    .focus_group();

    let body = match s.preferences_section {
        PreferencesSection::Appearance => appearance(s, k),
        PreferencesSection::Controls => controls(s, k),
        PreferencesSection::Performance => performance(s, k),
        PreferencesSection::Updates => updates(s, k),
    };

    col()
        .key("preferences")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(
            row()
                .key("preferences-nav-row")
                .w_full()
                .justify_center()
                .px(24.0)
                .pt(18.0)
                .pb(10.0)
                .child(nav),
        )
        .child(
            col()
                .key("preferences-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .items_center()
                .child(
                    col()
                        .key("preferences-content")
                        .w_full()
                        .max_w(720.0)
                        .gap(18.0)
                        .px(28.0)
                        .pt(4.0)
                        .pb(24.0)
                        .child(body),
                ),
        )
        .child(Divider::horizontal().build(k))
        .child(footer(k))
}

fn section(
    k: &Kit,
    key: &str,
    title: &str,
    glyph: &'static rune::Icon,
    description: &str,
    body: Vec<El>,
) -> El {
    col()
        .key(key)
        .w_full()
        .gap(14.0)
        .child(
            col()
                .key("heading")
                .w_full()
                .gap(2.0)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(icon(glyph, 16.0).color(k.c.icon_muted).a11y_hidden())
                        .child(
                            text(title)
                                .role("heading")
                                .level(2)
                                .font(k.ty.body_medium)
                                .color(k.c.text_base),
                        ),
                )
                .child(
                    text(description)
                        .font(k.ty.caption())
                        .color(k.c.text_muted)
                        .wrap(),
                ),
        )
        .children(body)
}

/// A titled card whose rows are separated by kit dividers. The subhead sits below the section
/// heading, so the page keeps a clear three-level hierarchy.
fn group(k: &Kit, key: &str, title: &str, rows: Vec<El>) -> El {
    let mut card = Card::new().child(
        text(title)
            .key("group-title")
            .role("heading")
            .level(3)
            .w_full()
            .pb(6.0)
            .font(k.ty.small_medium)
            .color(k.c.text_muted),
    );
    card = card.child(Divider::horizontal().build(k));
    for (index, entry) in rows.into_iter().enumerate() {
        if index > 0 {
            card = card.child(Divider::horizontal().build(k));
        }
        card = card.child(entry);
    }
    card.build(k)
        .key(key)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_base)
        .radius(k.radius.lg)
}

/// One setting: a label and explanation on the left, the control on the right. The visual copy
/// is hidden from accessibility because the control owns its name and description.
fn setting(k: &Kit, key: &str, label: &str, description: &str, control: El) -> El {
    row()
        .key(format!("setting-{key}"))
        .w_full()
        .gap(20.0)
        .center_y()
        .py(6.0)
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(label)
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base)
                        .a11y_hidden(),
                )
                .child(
                    text(description)
                        .font(k.ty.caption())
                        .color(k.c.text_muted)
                        .wrap()
                        .a11y_hidden(),
                ),
        )
        .child(control)
}

fn toggle_setting(
    k: &Kit,
    key: &str,
    label: &str,
    description: &str,
    checked: bool,
    disabled: bool,
    toggle: impl Fn(&mut crate::app::App) + 'static,
) -> El {
    let control = Switch::new(checked)
        .label(label)
        .hide_label()
        .disabled(disabled)
        .on_toggle(toggle)
        .build(k)
        .description(description);
    setting(k, key, label, description, control)
}

fn appearance(s: &crate::app::App, k: &Kit) -> El {
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
        .on_pick(|s: &mut crate::app::App, index, _| {
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
    section(
        k,
        "appearance",
        "Appearance",
        Icon::Color.glyph(),
        "How the window looks around your schematic.",
        vec![group(
            k,
            "appearance-theme",
            "Interface theme",
            vec![setting(k, "theme", "Theme", theme_description, theme)],
        )],
    )
}

fn controls(s: &crate::app::App, k: &Kit) -> El {
    section(
        k,
        "controls",
        "Controls",
        Icon::Keyboard.glyph(),
        "Camera feel and every viewer action for the orbit preview and free-flight spectator cameras.",
        vec![
            camera(s, k),
            bindings(s, k, Mode::Preview),
            bindings(s, k, Mode::Spectator),
        ],
    )
}

fn camera(s: &crate::app::App, k: &Kit) -> El {
    let sensitivity = s.preferences.sensitivity;
    let fov = s.preferences.fov;
    let sensitivity_value = format!("{:.0}%", sensitivity * 100.0);
    let fov_value = format!("{fov:.0}°");
    group(
        k,
        "controls-camera",
        "Camera feel",
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
    )
}

fn bindings(s: &crate::app::App, k: &Kit, mode: Mode) -> El {
    let title = format!("{} camera", mode.title());
    let mut body = vec![
        text("Press a keycap, then press any key or modifier. Escape cancels; Backspace clears the binding.")
            .font(k.ty.small)
            .color(k.c.text_muted)
            .wrap()
            .py(4.0),
    ];
    body.extend(
        Action::all(mode)
            .iter()
            .map(|action| binding_row(s, k, *action)),
    );
    group(k, &format!("controls-{mode:?}"), &title, body)
}

fn binding_row(s: &crate::app::App, k: &Kit, action: Action) -> El {
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
    let border = if recording {
        k.c.border_focus
    } else {
        k.c.border_base
    };
    let mut control = el()
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
        .h(30.0)
        .px(10.0)
        .min_w(112.0)
        .center()
        .radius(k.radius.md)
        .fill(k.c.bg_base)
        .border(1.0, border)
        .focus_ring(2.0, 2.0, k.c.border_focus)
        .on_press_tx(move |s: &mut crate::app::App, tx: &mut Tx| {
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
        .on_physical_input(move |s: &mut crate::app::App, input, tx| {
            record_physical_input(s, input, tx)
        });
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
            Keybind::new(&keys).build(k)
        }
        .a11y_hidden(),
    );
    row()
        .key(format!("binding-{}", action.key()))
        .w_full()
        .gap(16.0)
        .center_y()
        .py(2.0)
        .child(
            text(action.title())
                .grow(1.0)
                .font(k.ty.ui)
                .color(k.c.text_base)
                .a11y_hidden(),
        )
        .child(control)
}

/// Records a key the settings page receives before focus navigation. Returns true while
/// recording, so the viewer never sees a key meant for the recorder.
pub fn record_physical_input(s: &mut crate::app::App, input: &PhysicalInput, tx: &mut Tx) -> bool {
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
pub fn recording_key(s: &mut crate::app::App, event: &KeyEvent, tx: &mut Tx) -> bool {
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

fn assign(s: &mut crate::app::App, action: Action, key: KeyChord, tx: &mut Tx) {
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

fn cancel(s: &mut crate::app::App, action: Action, tx: &mut Tx) {
    s.recording = None;
    s.notice = None;
    tx.announce(
        format!("{} recording cancelled.", action.title()),
        LiveRegion::Polite,
    );
}

fn clear(s: &mut crate::app::App, action: Action, tx: &mut Tx) {
    s.preferences.clear_binding(action);
    s.recording = None;
    let message = format!("{} cleared.", action.title());
    s.notice = Some(message.clone());
    s.save_preferences();
    tx.announce(message, LiveRegion::Polite);
}

fn performance(s: &crate::app::App, k: &Kit) -> El {
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
        |s: &mut crate::app::App| {
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
        .label("Chunk size")
        .on_pick(|s: &mut crate::app::App, index, _| {
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
    let chunk_size = chunk_size
        .build(k)
        .description("The edge length, in blocks, of each geometry chunk.")
        .focus_group();
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
        |s: &mut crate::app::App| {
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
    let reset = row().w_full().justify_end().py(2.0).child(
        Button::new("Reset preview settings")
            .icon(Icon::Refresh.glyph())
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Mini)
            .on_press(|s: &mut crate::app::App| {
                s.preferences.preview = PreviewSettings::default();
                s.notice = Some("Preview settings reset.".to_owned());
                s.save_preferences();
            })
            .build(k)
            .key("reset-preview-settings"),
    );
    section(
        k,
        "performance",
        "Performance",
        Icon::Gauge.glyph(),
        "How the preview pipeline uses memory and CPU. Changes apply the next time you open a schematic.",
        vec![group(
            k,
            "preview",
            "Preview preparation",
            vec![
                chunking,
                setting(
                    k,
                    "chunk-size",
                    "Chunk size",
                    "The edge length, in blocks, of each geometry chunk.",
                    chunk_size,
                ),
                multithreading,
                thread_count,
                reset,
            ],
        )],
    )
}

const APPIMAGE_UPDATE_URL: &str = "https://github.com/AppImageCommunity/AppImageUpdate/issues";

fn updates(s: &crate::app::App, k: &Kit) -> El {
    let mut body = Vec::new();
    let title = match &s.update_mode {
        crate::update::Mode::SelfUpdate => "Automatic updates",
        crate::update::Mode::AppImage => "AppImage updates",
        _ => "Managed updates",
    };
    match &s.update_mode {
        crate::update::Mode::SelfUpdate => {
            body.push(toggle_setting(
                k,
                "auto-update",
                "Check for updates automatically",
                "When off, no update check runs at launch and the download source is hidden.",
                s.preferences.auto_update,
                false,
                |s: &mut crate::app::App| {
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
                    .on_pick(|s: &mut crate::app::App, index, _| {
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
                body.push(setting(
                    k,
                    "source",
                    "Update source",
                    source_description,
                    source,
                ));
                if let Some(status) = update_status(s, k) {
                    body.push(status);
                }
                let in_flight = matches!(
                    &s.update_state,
                    UpdateState::Checking | UpdateState::Installing
                );
                let mut actions = row().w_full().gap(8.0).center_y().child(
                    Button::new("Check for updates")
                        .icon(Icon::Refresh.glyph())
                        .disabled(in_flight)
                        .on_press(|s: &mut crate::app::App| s.check_update())
                        .build(k)
                        .key("check-updates"),
                );
                if matches!(&s.update_state, UpdateState::Available(_)) {
                    actions = actions.child(
                        Button::new("Install update")
                            .icon(Icon::ArrowDownload.glyph())
                            .variant(ButtonVariant::Contrast)
                            .on_press(|s: &mut crate::app::App| s.install_update())
                            .build(k)
                            .key("install-update"),
                    );
                }
                body.push(actions);
            } else {
                body.push(
                    text("Automatic updates are off. No update check or install action runs.")
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .wrap()
                        .py(4.0),
                );
            }
        }
        crate::update::Mode::AppImage => {
            if let Some(status) = update_status(s, k) {
                body.push(status);
            }
            body.push(
                text("This build is an AppImage. Updates are installed with AppImageUpdate; the download source is not configurable here.")
                    .font(k.ty.ui)
                    .color(k.c.text_muted)
                    .wrap()
                    .py(4.0),
            );
            body.push(
                Button::new("Check with AppImageUpdate")
                    .icon(Icon::ArrowDownload.glyph())
                    .on_press_tx(|_: &mut crate::app::App, tx: &mut Tx| {
                        let _ = tx.open_url(APPIMAGE_UPDATE_URL);
                    })
                    .build(k)
                    .key("check-appimage-update"),
            );
        }
        _ => {
            body.push(
                text("Updates are managed by your package manager. Update actions and the download source are unavailable in this build.")
                    .font(k.ty.ui)
                    .color(k.c.text_muted)
                    .wrap()
                    .py(4.0),
            );
        }
    }
    section(
        k,
        "updates",
        "Updates",
        Icon::ArrowDownload.glyph(),
        "How Litematica Preview checks for and installs new versions.",
        vec![group(k, "updates-card", title, body)],
    )
}

fn update_status(s: &crate::app::App, k: &Kit) -> Option<El> {
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
            .gap(6.0)
            .center_y()
            .py(4.0)
            .child(icon(glyph, 14.0).color(color).a11y_hidden())
            .child(text(message).font(k.ty.ui).color(color).wrap().grow(1.0)),
    )
}

fn footer(k: &Kit) -> El {
    row()
        .key("preferences-footer")
        .w_full()
        .justify_center()
        .px(28.0)
        .py(12.0)
        .child(
            row()
                .w_full()
                .max_w(720.0)
                .gap(16.0)
                .justify_between()
                .center_y()
                .child(
                    text("Settings are saved automatically.")
                        .font(k.ty.caption())
                        .color(k.c.text_muted),
                )
                .child(
                    Button::new("Reset all settings")
                        .icon(Icon::Refresh.glyph())
                        .variant(ButtonVariant::Outline)
                        .on_press(|s: &mut crate::app::App| {
                            s.preferences.reset();
                            s.preferences_section = PreferencesSection::default();
                            s.recording = None;
                            s.notice = Some("Settings reset to defaults.".to_owned());
                            s.save_preferences();
                        })
                        .build(k)
                        .key("reset-settings"),
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
    lower: impl Fn(&mut crate::app::App) + 'static,
    higher: impl Fn(&mut crate::app::App) + 'static,
) -> El {
    let down = Icon::ChevronDown.glyph();
    let up = down;
    let minus = Button::new(decrease)
        .label_with(move |_, ink| icon(down, 12.0).color(ink))
        .control_size(ControlSize::Mini)
        .help(decrease)
        .disabled(disabled)
        .on_press(move |s: &mut crate::app::App| {
            lower(s);
            s.save_preferences();
        })
        .build(k);
    let plus = Button::new(increase)
        .label_with(move |_, ink| icon(up, 12.0).color(ink).rotate(std::f32::consts::PI))
        .control_size(ControlSize::Mini)
        .help(increase)
        .disabled(disabled)
        .on_press(move |s: &mut crate::app::App| {
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
    setting(k, key, label, &description, control)
}
