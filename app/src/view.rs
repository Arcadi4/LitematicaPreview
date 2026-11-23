use rune::{Cx, El, Phase, PointerButton, Tx, col, el, icon, row, text};
use rune_kit::shell::{
    Page as ShellPage, Sidebar, SidebarHeader, SidebarRow, SidebarToggle, SplitView, TabItem,
    TabOrientation, TabOverflow, TabStrip, Toolbar,
};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Spinner, Switch, Toast,
    ToastRegion,
};

use crate::app::{App, Capture, Page, Preview, VIEWPORT, title};
use crate::formats;
use crate::icons::Icon;
use crate::preferences::Action;
use crate::{preferences_ui, preview, update};

pub(crate) const SCENE_EFFECT: u32 = 1;
/// Rune Kit's tab pill radius, so the active tab's wash sits flush in its surface.
const TAB_RADIUS: f32 = 6.0;

const OPEN_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "⌘O"
} else {
    "Ctrl+O"
};
const OPEN_FOLDER_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "⇧⌘O"
} else {
    "Ctrl+Shift+O"
};
const SETTINGS_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "⌘,"
} else {
    "Ctrl+,"
};

pub(crate) fn view(s: &App, cx: &mut Cx<App>) -> El {
    let kit = Kit::neutral(s.preferences.theme.is_dark(cx.system_dark()));
    let base = match s.page {
        Page::Viewer => split(s, cx, &kit),
        Page::Preferences => preferences(s, cx, &kit),
        Page::TexturePacks => page_shell(cx, &kit, "Texture Packs", texture_packs(s, &kit)),
    };
    let mut base = base
        .on_key_up(App::key_up)
        .on_drop_hover(|s: &mut App, inside, _| s.dropping = inside)
        .on_drop_files(|s: &mut App, paths, _| {
            s.dropping = false;
            s.open_paths(paths.to_vec());
        });
    if s.dropping {
        base = base.child(
            col()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .center()
                .gap(10.0)
                .inert()
                .fill(kit.alpha(kit.c.bg_base, 0.94))
                .child(
                    icon(Icon::OpenFiles.glyph(), 28.0)
                        .color(kit.c.icon_muted)
                        .a11y_hidden(),
                )
                .child(
                    text("Drop schematics to open them")
                        .font(kit.ty.body_medium)
                        .color(kit.c.text_base),
                )
                .child(
                    text("They open in a tab beside this one")
                        .font(kit.ty.small)
                        .color(kit.c.text_muted),
                ),
        );
    }
    rune_kit::root(base, [toasts(s, &kit)])
}

fn split(s: &App, cx: &mut Cx<App>, kit: &Kit) -> El {
    let main = viewer(s, kit);
    let mut toolbar = Toolbar::new();
    if !s.tabs.is_empty() {
        let mut strip = tabs(s, cx, kit);
        if s.sidebar.is_open() {
            // SplitView pads its toolbar 12 past the sidebar, where the panel sits flush.
            strip = strip.ml(-12.0);
            if let Some([width, _]) = cx.layout_size("main-pane") {
                strip = strip.w(width).min_w(width);
            }
        }
        toolbar = toolbar.leading(strip);
    }
    if !s.sidebar.is_open() {
        toolbar = toolbar
            .trailing(
                IconButton::new(Icon::BoxMultiple.glyph())
                    .variant(IconButtonVariant::GhostMuted)
                    .label("Texture Packs")
                    .help("Manage texture packs")
                    .on_press(|s: &mut App| s.open_page(Page::TexturePacks))
                    .build(kit)
                    .key("texture-packs"),
            )
            .trailing(
                IconButton::new(Icon::Settings.glyph())
                    .variant(IconButtonVariant::GhostMuted)
                    .label("Preferences")
                    .help("Open preferences")
                    .shortcut(SETTINGS_SHORTCUT)
                    .on_press(|s: &mut App| s.open_page(Page::Preferences))
                    .build(kit)
                    .key("preferences"),
            );
    }
    let sidebar = sidebar(s, kit);
    let mut split = SplitView::new(main)
        .sidebar(&s.sidebar, |s: &mut App| &mut s.sidebar, sidebar)
        .toolbar(toolbar)
        .titlebar_inset(cx.titlebar_inset())
        .frame_inset(8.0)
        .sidebar_padding(16.0, 12.0)
        .sidebar_toggle(
            SidebarToggle::new()
                .size(ControlSize::Large)
                .edge(16.0)
                .gap(4.0)
                .in_header()
                .shortcut(""),
        )
        .reduced_motion(cx.is_reduced_motion());
    if s.tabs.is_empty() {
        split = split.toolbar_in_panel();
    }
    split.build(kit)
}

fn viewer(s: &App, kit: &Kit) -> El {
    let content = if s.active.is_none() {
        welcome(s, kit)
    } else {
        stage(s, kit)
    };
    let mut viewer = col()
        .key("viewer")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(content);
    if s.active.is_some() {
        viewer = viewer.child(
            stage_caption(s, kit)
                .unwrap_or_else(|| preview_metadata(s, kit))
                .absolute(12.0, 12.0),
        );
    }
    viewer
}

fn tabs(s: &App, cx: &mut Cx<App>, kit: &Kit) -> El {
    let active = s.active_tab().map(|tab| tab.tab_token.as_str());
    let (surface, edge) = (kit.c.bg_base, kit.c.border_muted);
    let items = s.tabs.iter().map(|tab| {
        let label = title(&tab.path);
        let help = tab.path.display().to_string();
        let key = format!("tab-{}", tab.id);
        let mut item = TabItem::new(tab.tab_token.as_str(), label).closable(true);
        if let Some(image) = s.thumbnails.get(&tab.path) {
            item = item.avatar(thumbnail(image));
        } else {
            item = item.icon(Icon::Cube.glyph());
        }
        item.decorate(move |slot, _| {
            slot.key(key)
                .help(help)
                .radius(TAB_RADIUS)
                .fill(surface)
                .border(1.0, edge)
                .clip_rounded(TAB_RADIUS)
        })
    });
    TabStrip::new("tab-strip", TabOrientation::Horizontal)
        .tabs(items)
        .active(active)
        .viewport_width(cx.layout_size("tab-strip").map(|size| size[0]))
        .overflow(TabOverflow::ShrinkThenScroll {
            min_width: 104.0,
            max_width: 164.0,
            pinned_width: 36.0,
            fade_width: 16.0,
        })
        .gap(4.0)
        .row_height(28.0)
        .close_tab_stop(true)
        .fill(surface)
        .close_icon(Icon::Dismiss.glyph())
        .label("Open schematics")
        .on_select(|s: &mut App, id: &str, tx: &mut Tx| {
            if let Ok(id) = id.parse::<u64>() {
                s.activate(id, tx);
            }
        })
        .on_close(|s: &mut App, id: &str, tx: &mut Tx| {
            if let Ok(id) = id.parse::<u64>() {
                s.close_tab(id, tx);
            }
        })
        .build(kit)
}

fn welcome(s: &App, kit: &Kit) -> El {
    let extensions = formats::EXTENSIONS.join("  ·  ");
    let mark = el()
        .w(44.0)
        .h(44.0)
        .radius(12.0)
        .fill(kit.c.layer_02)
        .center()
        .child(
            icon(Icon::Cube.glyph(), 24.0)
                .color(kit.c.text_base)
                .a11y_hidden(),
        );
    let actions = row()
        .gap(8.0)
        .center_y()
        .child(
            Button::new("Open Files…")
                .icon(Icon::OpenFiles.glyph())
                .variant(ButtonVariant::Contrast)
                .control_size(ControlSize::Large)
                .shortcut("cmd+o")
                .disabled(s.choosing)
                .on_press(|s: &mut App| s.open_dialog())
                .build(kit)
                .key("welcome-open-files"),
        )
        .child(
            Button::new("Open Folder…")
                .icon(Icon::OpenFolder.glyph())
                .control_size(ControlSize::Large)
                .shortcut("cmd+shift+o")
                .disabled(s.choosing)
                .on_press_tx(App::open_folder)
                .build(kit)
                .key("welcome-open-folder"),
        );
    let mut content = col()
        .w_full()
        .max_w(640.0)
        .gap(20.0)
        .items_center()
        .child(mark)
        .child(
            col()
                .gap(6.0)
                .items_center()
                .child(
                    text("Litematica Preview")
                        .font(rune::TextStyle {
                            size: 22.0,
                            weight: 600.0,
                            line_height: 28.0,
                            ..kit.ty.ui_medium
                        })
                        .color(kit.c.text_base),
                )
                .child(
                    text("Drop schematics anywhere to preview them offline")
                        .font(kit.ty.ui)
                        .color(kit.c.text_muted),
                ),
        )
        .child(actions)
        .child(text(extensions).font(kit.ty.small).color(kit.c.text_faint));
    if !s.demos.is_empty() {
        let demos = s.demos.clone();
        content = content.child(
            Button::new("Try the examples")
                .icon(Icon::BoxMultiple.glyph())
                .variant(ButtonVariant::GhostMuted)
                .on_press(move |s: &mut App| s.open_paths(demos.clone()))
                .build(kit)
                .key("welcome-examples"),
        );
    }
    col()
        .key("welcome")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .padding(24.0)
        .center()
        .child(content)
        .child(
            text(format!("Version {}", env!("CARGO_PKG_VERSION")))
                .font(kit.ty.small)
                .color(kit.c.text_faint)
                .mt(24.0),
        )
}

fn sidebar(s: &App, kit: &Kit) -> El {
    let mut sidebar = Sidebar::new()
        .label("Nearby schematics")
        .inset(0.0)
        .header_gap(2.0)
        .header_padding(0.0, 8.0)
        .row_gap(2.0);
    let nearby = s.nearby();
    if let Some((all, index)) = nearby {
        let (label, detail) = s.nearby_directory().map_or_else(
            || {
                (
                    "Selected files".into(),
                    "Schematics from multiple folders".into(),
                )
            },
            |folder| {
                (
                    folder
                        .file_name()
                        .unwrap_or(folder.as_os_str())
                        .to_string_lossy(),
                    folder.to_string_lossy(),
                )
            },
        );
        sidebar = sidebar.header(
            SidebarHeader::new(label.into_owned())
                .count(all.len())
                .build(kit)
                .help(detail),
        );
        let rows = kit.clone();
        sidebar = sidebar.row_height(32.0).virtual_rows(
            all.len(),
            |_: &App, at| format!("browse-{at}").into(),
            move |s: &App, at| browse_row(s, at, &rows),
        );
        if all.len() > 1 {
            sidebar = sidebar.footer(nearby_nav(index, all.len(), kit));
        }
    } else {
        sidebar = sidebar
            .header(
                SidebarRow::new("Open Files…")
                    .icon(Icon::OpenFiles.glyph())
                    .detail(OPEN_SHORTCUT)
                    .height(32.0)
                    .on_select(|s: &mut App, _| s.open_dialog())
                    .build(kit)
                    .key("sidebar-open-files"),
            )
            .header(
                SidebarRow::new("Open Folder…")
                    .icon(Icon::OpenFolder.glyph())
                    .detail(OPEN_FOLDER_SHORTCUT)
                    .height(32.0)
                    .on_select(|s: &mut App, tx| s.open_folder(tx))
                    .build(kit)
                    .key("sidebar-open-folder"),
            )
            .header(
                SidebarRow::new("Home")
                    .icon(Icon::Home.glyph())
                    .height(32.0)
                    .on_select(|s: &mut App, _| s.home())
                    .build(kit)
                    .key("sidebar-home"),
            );
    }
    sidebar = sidebar
        .footer(
            SidebarRow::new("Texture Packs")
                .icon(Icon::BoxMultiple.glyph())
                .height(32.0)
                .on_select(|s: &mut App, _| s.open_page(Page::TexturePacks))
                .build(kit)
                .key("sidebar-texture-packs"),
        )
        .footer(
            SidebarRow::new("Preferences")
                .icon(Icon::Settings.glyph())
                .detail(SETTINGS_SHORTCUT)
                .height(32.0)
                .on_select(|s: &mut App, _| s.open_page(Page::Preferences))
                .build(kit)
                .key("sidebar-preferences"),
        );
    col()
        .key("schematic-browser")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(sidebar.build(kit))
}

fn browse_row(s: &App, at: usize, kit: &Kit) -> El {
    let Some((paths, index)) = s.nearby() else {
        return col();
    };
    let Some(path) = paths.get(at) else {
        return col();
    };
    let mut row = SidebarRow::new(title(path))
        .height(30.0)
        .selected(at == index)
        .on_select(move |s: &mut App, tx: &mut Tx| s.browse_at(at, tx));
    if let Some(image) = s.thumbnails.get(path) {
        row = row.avatar(thumbnail(image));
    } else {
        row = row.icon(Icon::Cube.glyph());
    }
    col()
        .w_full()
        .h(32.0)
        .pb(2.0)
        .child(row.build(kit).w_full().help(path.display().to_string()))
}

fn nearby_nav(index: usize, count: usize, kit: &Kit) -> El {
    let previous = Button::new("Previous")
        .variant(ButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .help("Open the previous schematic")
        .disabled(index == 0)
        .on_press_tx(move |s: &mut App, tx: &mut Tx| s.browse_at(index.saturating_sub(1), tx))
        .build(kit)
        .key("browse-previous");
    let next = Button::new("Next")
        .variant(ButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .help("Open the next schematic")
        .disabled(index + 1 >= count)
        .on_press_tx(move |s: &mut App, tx: &mut Tx| s.browse_at(index + 1, tx))
        .build(kit)
        .key("browse-next");
    row()
        .key("browse-nav")
        .w_full()
        .h(28.0)
        .px(2.0)
        .gap(4.0)
        .center_y()
        .child(previous)
        .child(
            text(format!("{} of {}", index + 1, count))
                .font(kit.ty.small)
                .color(kit.c.text_faint)
                .grow(1.0),
        )
        .child(next)
}

fn stage(s: &App, kit: &Kit) -> El {
    let model = rune::effect(SCENE_EFFECT)
        .key(VIEWPORT)
        .label("Schematic viewport")
        .role("region")
        .focusable(true)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .pointer_buttons([PointerButton::Middle])
        .on_drag(|s: &mut App, drag, tx| {
            let now = [drag.dx, drag.dy];
            match drag.phase {
                Phase::Down => {
                    s.drag_previous = now;
                    tx.focus(VIEWPORT, false);
                }
                Phase::Move => {
                    let [dx, dy] = [now[0] - s.drag_previous[0], now[1] - s.drag_previous[1]];
                    s.drag_previous = now;
                    let pan = tx.button() == PointerButton::Middle || tx.mods().shift;
                    if !s.spectator
                        && let Some(camera) = s.camera_mut()
                    {
                        if pan {
                            camera.pan(dx, dy);
                        } else {
                            camera.orbit(dx, dy);
                        }
                    }
                }
                Phase::Up => s.drag_previous = [0.0; 2],
            }
        })
        .on_pointer_move_local(|s: &mut App, point, _| {
            if !s.spectator {
                return;
            }
            let now = [point.x, point.y];
            if let Some(last) = s.look_previous.replace(now) {
                let sensitivity = s.preferences.sensitivity;
                if let Some(camera) = s.camera_mut() {
                    camera.fly_look(
                        (now[0] - last[0]) * sensitivity,
                        (now[1] - last[1]) * sensitivity,
                    );
                }
            }
        })
        .on_wheel(|s: &mut App, wheel, _| {
            if s.spectator {
                return false;
            }
            if let Some(camera) = s.camera_mut() {
                camera.zoom_wheel(wheel.dy, 0);
            }
            true
        })
        .on_magnify(|s: &mut App, event, _| {
            if s.spectator {
                return false;
            }
            if let Some(camera) = s.camera_mut() {
                camera.zoom((1.0 + event.delta).recip());
            }
            true
        });
    col()
        .key("preview")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(model)
        .child(export_controls(s, kit))
        .child(
            row()
                .key("preview-footer")
                .pin(Some(8.0), None, Some(12.0), Some(8.0))
                .gap(12.0)
                .center_y()
                .child(view_controls(s, kit)),
        )
}

fn stage_caption(s: &App, kit: &Kit) -> Option<El> {
    let mut caption = row()
        .padding(8.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .fill(kit.c.bg_base)
        .border(1.0, kit.c.border_base);
    match &s.preview {
        Preview::Loading(phase, done, total) => {
            let status = match *phase {
                "decode" => "Reading schematic",
                "mesh" => "Building geometry",
                "upload" => "Uploading geometry",
                other => other,
            };
            let label = if *total == 0 {
                status.to_string()
            } else {
                format!("{status} {done}/{total}")
            };
            caption = caption
                .child(Spinner::new().size(12.0).color(kit.c.icon_muted).build(kit))
                .child(
                    text(label)
                        .key("load-progress")
                        .font(kit.ty.small)
                        .color(kit.c.text_base),
                )
                .child(
                    Button::new("Cancel")
                        .variant(ButtonVariant::GhostMuted)
                        .control_size(ControlSize::Small)
                        .on_press_tx(|s: &mut App, tx| {
                            if let Some(id) = s.active {
                                s.close_tab(id, tx);
                            }
                        })
                        .build(kit),
                );
        }
        Preview::Failed(error) => {
            caption = caption.max_w(360.0).child(
                text(error)
                    .key("preview-error")
                    .font(kit.ty.small)
                    .color(kit.c.text_base)
                    .wrap(),
            );
        }
        _ => return None,
    }
    Some(caption)
}

fn view_controls(s: &App, kit: &Kit) -> El {
    let loaded = matches!(s.preview, Preview::Ready(_));
    let shortcut = |action: Action| s.preferences.binding(action).label();
    row()
        .key("view-controls")
        .gap(2.0)
        .center_y()
        .child(
            IconButton::new(Icon::FullScreen.glyph())
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Fit")
                .help("Fit the schematic in view")
                .shortcut(shortcut(Action::Fit))
                .on_press(|s: &mut App| {
                    if let Some(camera) = s.camera_mut() {
                        camera.fit();
                    }
                })
                .build(kit)
                .key("fit"),
        )
        .child(
            IconButton::new(Icon::ZoomOut.glyph())
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Zoom out")
                .help("Zoom out")
                .shortcut(shortcut(Action::ZoomOut))
                .on_press(|s: &mut App| {
                    if let Some(camera) = s.camera_mut() {
                        camera.zoom(1.12);
                    }
                })
                .build(kit)
                .key("zoom-out"),
        )
        .child(
            IconButton::new(Icon::ZoomIn.glyph())
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Zoom in")
                .help("Zoom in")
                .shortcut(shortcut(Action::ZoomIn))
                .on_press(|s: &mut App| {
                    if let Some(camera) = s.camera_mut() {
                        camera.zoom(0.88);
                    }
                })
                .build(kit)
                .key("zoom-in"),
        )
        .child(
            IconButton::new(Icon::Grid.glyph())
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label(if s.grid { "Hide grid" } else { "Show grid" })
                .help(if s.grid {
                    "Hide the block grid"
                } else {
                    "Show the block grid"
                })
                .state(if s.grid {
                    rune_kit::State::Pressed
                } else {
                    rune_kit::State::Rest
                })
                .on_press(|s: &mut App| s.toggle_grid())
                .build(kit)
                .key("toggle-grid"),
        )
        .child(
            IconButton::new(Icon::Person.glyph())
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label(if s.spectator {
                    "Exit spectator mode"
                } else {
                    "Spectator mode"
                })
                .help(if s.spectator {
                    "Return to orbiting"
                } else {
                    "Move the pointer to look around"
                })
                .shortcut(shortcut(Action::ToggleSpectator))
                .state(if s.spectator {
                    rune_kit::State::Pressed
                } else {
                    rune_kit::State::Rest
                })
                .disabled(!loaded)
                .on_press_tx(App::toggle_flying)
                .build(kit)
                .key("spectator"),
        )
}

fn export_controls(s: &App, kit: &Kit) -> El {
    let loaded = matches!(s.preview, Preview::Ready(_));
    row()
        .key("export-controls")
        .pin(None, Some(8.0), Some(8.0), None)
        .gap(2.0)
        .center_y()
        .child(
            IconButton::new(Icon::Copy.glyph())
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Copy image")
                .help("Copy the preview as an image")
                .disabled(!loaded)
                .on_press(|s: &mut App| s.capture = Some(Capture::Copy))
                .build(kit)
                .key("copy-image"),
        )
        .child(
            IconButton::new(Icon::Save.glyph())
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Save image…")
                .help("Save the preview as an image")
                .disabled(!loaded)
                .on_press_tx(App::save_image)
                .build(kit)
                .key("save-image"),
        )
}

fn preview_metadata(s: &App, kit: &Kit) -> El {
    let counts = match &s.preview {
        Preview::Ready(data) => format!(
            "{} blocks  ·  {} triangles",
            data.block_count, data.triangle_count
        ),
        Preview::Loading(..) => "Loading…".into(),
        Preview::Failed(_) => "Preview unavailable".into(),
        Preview::None => "Preparing preview…".into(),
    };
    text(counts)
        .key("preview-metadata")
        .font(kit.ty.small)
        .color(kit.c.text_weak)
}

fn toasts(s: &App, kit: &Kit) -> El {
    let mut region = ToastRegion::new();
    if let Some(notice) = &s.notice {
        region = region.toast(
            Toast::new("notice", notice.as_str())
                .on_close(|s: &mut App| s.notice = None)
                .build(kit),
        );
    }
    if s.page == Page::Viewer
        && matches!(s.update_state, update::State::Available(_))
        && s.preferences.auto_update
    {
        region = region.toast(
            Toast::new("update-available", "An update is available.")
                .action(kit, "View update", true, |s: &mut App, _: &mut Tx| {
                    s.preferences_section = preferences_ui::PreferencesSection::Updates;
                    s.open_page(Page::Preferences);
                })
                .build(kit),
        );
    }
    region.build(kit)
}

/// The preferences page keeps the viewer's frame: its sections take the window's sidebar, at
/// the sidebar's width, shown even where the viewer hides it.
fn preferences(s: &App, cx: &mut Cx<App>, kit: &Kit) -> El {
    let mut pane = s.sidebar;
    pane.set_open(true);
    SplitView::new(preferences_ui::view(s, kit))
        .sidebar(
            &pane,
            |s: &mut App| &mut s.sidebar,
            preferences_ui::sidebar(s, kit),
        )
        .toolbar(
            Toolbar::new()
                .leading(back(kit))
                .title(s.preferences_section.title()),
        )
        .titlebar_inset(cx.titlebar_inset())
        .frame_inset(8.0)
        .sidebar_padding(16.0, 12.0)
        .hide_sidebar_toggle()
        .reduced_motion(cx.is_reduced_motion())
        .build(kit)
}

fn page_shell(cx: &mut Cx<App>, kit: &Kit, title: &str, body: El) -> El {
    let content = col()
        .key("page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(col().w_full().grow(1.0).min_h(0.0).child(body));
    ShellPage::new(content)
        .title(title)
        .leading(back(kit))
        .titlebar_inset(cx.titlebar_inset())
        .frame_inset(8.0)
        .build(kit)
}

fn back(kit: &Kit) -> El {
    Button::new("Back")
        .icon(Icon::Back.glyph())
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Regular)
        .help("Back to the preview")
        .on_press(|s: &mut App| s.open_page(Page::Viewer))
        .build(kit)
        .key("back")
}

fn texture_packs(s: &App, kit: &Kit) -> El {
    let mut content = col().w_full().max_w(640.0).gap(12.0).child(
        text("Enabled packs are applied from top to bottom. Bundled textures are used for missing blocks.")
            .font(kit.ty.small)
            .color(kit.c.text_muted)
            .wrap()
            .w_full(),
    );
    content = content.child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(
                Button::new("Import Pack…")
                    .icon(Icon::OpenFiles.glyph())
                    .disabled(s.pack_busy)
                    .on_press_tx(App::show_texture_picker)
                    .build(kit)
                    .key("import-pack"),
            )
            .child(el().grow(1.0).a11y_hidden()),
    );
    if s.packs.is_empty() {
        content = content.child(
            col()
                .w_full()
                .padding(18.0)
                .gap(4.0)
                .center()
                .radius(kit.radius.lg)
                .fill(kit.c.layer_01)
                .border(1.0, kit.c.border_base)
                .child(
                    text("No texture packs installed")
                        .font(kit.ty.ui_medium)
                        .color(kit.c.text_base),
                )
                .child(
                    text("Import a resource pack ZIP to use its blocks.")
                        .font(kit.ty.small)
                        .color(kit.c.text_muted),
                ),
        );
    }
    for (index, pack) in s.packs.iter().enumerate() {
        let id = pack.id.clone();
        let name = pack.name.clone();
        let remove_id = id.clone();
        let mut order: Vec<_> = s.packs.iter().map(|pack| pack.id.clone()).collect();
        let mut down = order.clone();
        if index > 0 {
            order.swap(index, index - 1);
        }
        if index + 1 < down.len() {
            down.swap(index, index + 1);
        }
        content = content.child(
            row()
                .key(&pack.id)
                .w_full()
                .padding(10.0)
                .gap(8.0)
                .center_y()
                .radius(kit.radius.lg)
                .fill(kit.c.layer_01)
                .border(1.0, kit.c.border_base)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text(&name)
                                .font(kit.ty.ui)
                                .color(kit.c.text_base)
                                .truncate(),
                        )
                        .child(
                            text(format!("Priority {}", index + 1))
                                .font(kit.ty.small)
                                .color(kit.c.text_faint),
                        ),
                )
                .child(
                    Switch::new(pack.enabled)
                        .label(name.clone())
                        .disabled(s.pack_busy)
                        .on_toggle(move |s: &mut App| {
                            let id = id.clone();
                            let enabled = !s.packs[index].enabled;
                            s.change_packs(move |lib| {
                                lib.set_enabled(&id, enabled, |layers| {
                                    preview::validate_texture_library(layers, |layer| {
                                        std::fs::read(&layer.path).map_err(|e| e.to_string())
                                    })
                                })
                                .map(|_| ())
                            });
                        })
                        .build(kit),
                )
                .child(
                    IconButton::new(Icon::ArrowUp.glyph())
                        .variant(IconButtonVariant::GhostMuted)
                        .control_size(ControlSize::Small)
                        .label(format!("Move {name} up"))
                        .disabled(index == 0 || s.pack_busy)
                        .on_press(move |s: &mut App| {
                            let order = order.clone();
                            s.change_packs(move |lib| lib.set_order(&order).map(|_| ()));
                        })
                        .build(kit),
                )
                .child(
                    IconButton::new(Icon::ChevronDown.glyph())
                        .variant(IconButtonVariant::GhostMuted)
                        .control_size(ControlSize::Small)
                        .label(format!("Move {name} down"))
                        .disabled(index + 1 == s.packs.len() || s.pack_busy)
                        .on_press(move |s: &mut App| {
                            let down = down.clone();
                            s.change_packs(move |lib| lib.set_order(&down).map(|_| ()));
                        })
                        .build(kit),
                )
                .child(
                    IconButton::new(Icon::Delete.glyph())
                        .variant(IconButtonVariant::GhostMuted)
                        .control_size(ControlSize::Small)
                        .label(format!("Remove {}", s.packs[index].name))
                        .disabled(s.pack_busy)
                        .on_press(move |s: &mut App| {
                            let id = remove_id.clone();
                            s.change_packs(move |lib| lib.remove(&id).map(|_| ()));
                        })
                        .build(kit),
                ),
        );
    }
    col()
        .key("texture-packs")
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(16.0)
        .items_center()
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(content)
}

fn thumbnail(image: &rune::Image) -> El {
    el().size(16.0, 16.0)
        .radius(3.0)
        .pixels_fit(image.pixels(), rune::SurfaceFit::Fill)
}
