//! Editor UI
//!

use dear_imgui_rs::{Condition, StyleColor, TextureId, Ui, WindowFlags};
use kradiant::map::Map;

use crate::config::EditorConfig;
use crate::icons::EditorIcons;
use crate::images::EditorImages;
use crate::theme::{EditorPalette, ThemeEntry, theme_from_str};
use crate::util::text_width;
use crate::{EDITOR_THEMES, util};

pub mod console;
pub mod texbro;
pub mod undo;
pub mod view2d;
pub mod view3d;
use console::ConsoleLogger;
use texbro::TextureBrowser;
use undo::UndoRedo;
pub use view2d::{AxisLock, DragMode, Ortho, StretchMode, View2D};
use view3d::View3D;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FaceSelection {
    pub entity_idx: usize,
    pub brush_idx: usize,
    pub face_idx: usize,
}

/// Helper function to create an icon button with fallback text and tooltip.
fn icon_button(ui: &Ui, id: &str, icon: Option<TextureId>, tooltip: &str) -> bool {
    let clicked = if let Some(tid) = icon {
        ui.image_button_config(id, tid, [24.0, 24.0]).build()
    } else {
        ui.button(tooltip)
    };
    if ui.is_item_hovered() {
        ui.tooltip_text(tooltip);
    }
    clicked
}

/// Helper function to create a toggle button with icon and tint based on state.
fn icon_button_toggle(
    ui: &Ui,
    id: &str,
    icon: Option<TextureId>,
    fallback_label: &str,
    is_enabled: bool,
    tooltip: &str,
) -> bool {
    let tint_col = if is_enabled {
        [1.0, 1.0, 1.0, 1.0]
    } else {
        [1.0, 1.0, 1.0, 0.5]
    };
    let clicked = if let Some(tid) = icon {
        ui.image_button_config(id, tid, [24.0, 24.0])
            .tint_color(tint_col)
            .build()
    } else {
        ui.button(fallback_label)
    };
    if ui.is_item_hovered() {
        ui.tooltip_text(tooltip);
    }
    clicked
}

/// Helper function to check if a key combination was pressed.
fn key_combo_pressed(ui: &Ui, main_key: dear_imgui_rs::Key, modifiers: u32) -> bool {
    if !ui.is_key_pressed(main_key) {
        return false;
    }
    const CTRL: u32 = 1;
    const SHIFT: u32 = 2;

    let has_ctrl = ui.is_key_down(dear_imgui_rs::Key::LeftCtrl)
        || ui.is_key_down(dear_imgui_rs::Key::RightCtrl);
    let has_shift = ui.is_key_down(dear_imgui_rs::Key::LeftShift)
        || ui.is_key_down(dear_imgui_rs::Key::RightShift);

    let needs_ctrl = (modifiers & CTRL) != 0;
    let needs_shift = (modifiers & SHIFT) != 0;

    (has_ctrl == needs_ctrl) && (has_shift == needs_shift)
}

pub struct EditorState {
    pub config: EditorConfig,
    pub show_demo: bool,
    pub show_about: bool,
    pub toolbar_height: f32,
    pub map_path: String,
    pub map_revision: u64,
    pub undo: UndoRedo,
    pub view2d: View2D,
    pub view3d: View3D,
    pub stretch_mode: StretchMode,
    pub rotate_mode: bool,
    pub axis_lock: AxisLock,
    pub selection_rgba: [f32; 4],
    pub tex_browser: TextureBrowser,
    pub tex_filter: String,
    pub tex_selected: Option<String>,
    pub tex_tile_size: f32,
    pub console: ConsoleLogger,
    pub con_filter: String,
    pub map: Option<kradiant::map::Map>,
    pub selected_entity: Option<usize>,
    pub selected_brushes: Vec<(usize, usize)>,
    pub selected_faces: Vec<FaceSelection>,
    pub new_prop_key: String,
    pub new_prop_val: String,
    pub icons: EditorIcons,
    pub images: EditorImages,
    pub themes: Vec<ThemeEntry>,
    pub pending_theme: Option<usize>,
    pub palette: EditorPalette,
    pub edit_faces: bool,
    pub edit_edges: bool,
    pub edit_vertices: bool,
}

#[macro_export]
macro_rules! log_info {
    ($console:expr, $($arg:tt)+) => {
        $console.info(format!($($arg)+))
    };
}
#[macro_export]
macro_rules! log_warn {
    ($console:expr, $($arg:tt)+) => {
        $console.warn(format!($($arg)+))
    };
}
#[macro_export]
macro_rules! log_error {
    ($console:expr, $($arg:tt)+) => {
        $console.error(format!($($arg)+))
    };
}

impl Default for EditorState {
    fn default() -> Self {
        let themes: Vec<ThemeEntry> = EDITOR_THEMES
            .iter()
            .map(|theme_decl| {
                let theme_data = theme_from_str(theme_decl[1]).expect("Failed to parse theme");
                ThemeEntry {
                    name: theme_decl[0].to_string(),
                    data: theme_data,
                }
            })
            .collect();

        let mut s = Self {
            config: EditorConfig::default(),
            show_demo: false,
            show_about: false,
            toolbar_height: 0.0,
            map_path: String::new(),
            map_revision: 0,
            undo: UndoRedo::default(),
            view2d: View2D::default(),
            view3d: View3D::default(),
            stretch_mode: StretchMode::default(),
            rotate_mode: false,
            axis_lock: AxisLock::default(),
            selection_rgba: [0.3, 0.6, 1.0, 1.0],
            tex_browser: TextureBrowser::default(),
            tex_filter: String::new(),
            tex_selected: None,
            tex_tile_size: 64.0,
            console: ConsoleLogger::default(),
            con_filter: String::new(),
            map: Some(kradiant::map::Map::default()),
            selected_entity: None,
            selected_brushes: Vec::new(),
            selected_faces: Vec::new(),
            new_prop_key: String::new(),
            new_prop_val: String::new(),
            icons: EditorIcons::default(),
            images: EditorImages::default(),
            themes,
            pending_theme: None,
            palette: EditorPalette::default(),
            edit_faces: false,
            edit_edges: false,
            edit_vertices: false,
        };

        log_info!(s.console, "Kradiant editor started");

        match EditorConfig::load() {
            Ok(c) => s.config = c,
            Err(e) => log_error!(s.console, "Failed to load configuration: {}", e.to_string()),
        }

        log_warn!(s.console, "this is a warning");
        s
    }
}

// Top-level draw call

pub fn draw_editor(ui: &Ui, state: &mut EditorState, dt: f32) {
    draw_dockspace(ui, state);
    draw_main_menu(ui, state);
    draw_toolbar(ui, state);
    draw_entity_list(ui, state);
    draw_properties(ui, state);
    //draw_view3d(ui, state);
    {
        let view3d_ref = &mut state.view3d;
        let config = &mut state.config;
        let palette = &state.palette;

        view3d_ref.draw_impl(ui, config, palette);
    }

    state.selection_rgba = ui.style_color(StyleColor::ButtonActive);

    //  Call view2d draw separately to avoid dual mutable borrow
    {
        let view2d_ref = &mut state.view2d;
        let config = &mut state.config;
        let axis_lock = &state.axis_lock;
        let rotate_mode = state.rotate_mode;
        let stretch_mode = state.stretch_mode;
        let palette = &state.palette;
        let console = &mut state.console;
        let undo = &mut state.undo;
        let selected_brushes = &mut state.selected_brushes;
        let selected_faces = &mut state.selected_faces;
        let selected_entity = &mut state.selected_entity;
        let edit_faces = state.edit_faces;
        let map = &mut state.map;
        let selection_rgba = state.selection_rgba;

        view2d_ref.draw_impl(
            ui,
            config,
            axis_lock,
            rotate_mode,
            stretch_mode,
            palette,
            console,
            undo,
            selected_brushes,
            selected_faces,
            selected_entity,
            edit_faces,
            map,
            selection_rgba,
            dt,
        );
    }

    console::draw_console(ui, state);
    draw_texture_browser(ui, state);

    if state.show_demo {
        ui.show_demo_window(&mut state.show_demo);
    }

    // About dialog
    if state.show_about && !ui.is_popup_open("About Kradiant") {
        ui.open_popup("About Kradiant");
    }

    if let Some(_popup) = ui
        .begin_modal_popup_config("About Kradiant")
        .opened(&mut state.show_about)
        .flags(WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_COLLAPSE)
        .begin()
    {
        // image
        let logo_w = 256.0;
        util::center_next(ui, logo_w);
        if let Some(logo_tid) = state.images.splash {
            ui.image(logo_tid, [logo_w, 64.0]);
        }
        draw_about_dialog(ui);
    }
}

// Dockspace

fn draw_dockspace(ui: &Ui, state: &mut EditorState) {
    unsafe {
        let vp = dear_imgui_rs::sys::igGetMainViewport().as_ref().unwrap();
        //let menu_h = dear_imgui_rs::sys::igGetFrameHeight();
        let offset_y = state.toolbar_height;
        let pos = dear_imgui_rs::sys::ImVec2 {
            x: vp.WorkPos.x,
            y: vp.WorkPos.y + offset_y,
        };
        let size = dear_imgui_rs::sys::ImVec2 {
            x: vp.WorkSize.x,
            y: vp.WorkSize.y - offset_y,
        };
        dear_imgui_rs::sys::igSetNextWindowPos(
            pos,
            dear_imgui_rs::sys::ImGuiCond_Always as i32,
            dear_imgui_rs::sys::ImVec2 { x: 0.0, y: 0.0 },
        );
        dear_imgui_rs::sys::igSetNextWindowSize(size, dear_imgui_rs::sys::ImGuiCond_Always as i32);
        dear_imgui_rs::sys::igSetNextWindowBgAlpha(0.0);
    }

    let flags = WindowFlags::NO_DECORATION
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_NAV_FOCUS
        | WindowFlags::from_bits_truncate(1 << 13); // NoBringToDisplayFront

    ui.window("##dockspace_root").flags(flags).build(|| {
        let id = unsafe { dear_imgui_rs::sys::igGetID_Str(b"MainDockspace\0".as_ptr() as _) };

        // Build the default layout exactly once — before the DockSpace call so the
        // nodes exist when windows are first shown.
        build_default_layout(id);

        unsafe {
            // ImGuiDockNodeFlags_PassthruCentralNode = 1 << 3 = 8
            dear_imgui_rs::sys::igDockSpace(
                id,
                dear_imgui_rs::sys::ImVec2 { x: 0.0, y: 0.0 },
                8,
                std::ptr::null(),
            );
        }
    });
}

fn build_default_layout(dockspace_id: dear_imgui_rs::sys::ImGuiID) {
    use dear_imgui_rs::sys::*;

    unsafe {
        // Only initialise once — if the node already has children, the user has
        // already arranged the layout (or the ini file loaded it).
        if !igDockBuilderGetNode(dockspace_id).is_null()
            && (*igDockBuilderGetNode(dockspace_id)).ChildNodes[0] != std::ptr::null_mut()
        {
            return;
        }

        let vp = igGetMainViewport();
        let size = (*vp).WorkSize;

        // Start fresh.
        igDockBuilderRemoveNode(dockspace_id);
        // ImGuiDockNodeFlags_DockSpace = 1 << 10 = 1024
        igDockBuilderAddNode(dockspace_id, 1024);
        igDockBuilderSetNodeSize(dockspace_id, size);

        // Split root LEFT / RIGHT  (left ~58 % of width)
        let mut dock_right = 0u32;
        let mut dock_left = 0u32;
        igDockBuilderSplitNode(
            dockspace_id,
            ImGuiDir_Left,
            0.58,
            &mut dock_left,
            &mut dock_right,
        );

        // Split LEFT into TOP (2D View) and BOTTOM (Console)
        let mut dock_2d = 0u32;
        let mut dock_console = 0u32;
        igDockBuilderSplitNode(
            dock_left,
            ImGuiDir_Down,
            0.19, // console gets bottom ~19 %
            &mut dock_console,
            &mut dock_2d,
        );

        // Split RIGHT into TOP (3D View) and BOTTOM (tabs)
        let mut dock_3d = 0u32;
        let mut dock_tabs = 0u32;
        igDockBuilderSplitNode(
            dock_right,
            ImGuiDir_Down,
            0.19,
            &mut dock_tabs,
            &mut dock_3d,
        );

        // Dock windows
        igDockBuilderDockWindow(b"2D View\0".as_ptr() as _, dock_2d);
        igDockBuilderDockWindow(b"Console\0".as_ptr() as _, dock_console);
        igDockBuilderDockWindow(b"3D View\0".as_ptr() as _, dock_3d);
        // Three windows share the bottom-right node as tabs.
        igDockBuilderDockWindow(b"Textures\0".as_ptr() as _, dock_tabs);
        igDockBuilderDockWindow(b"Entities\0".as_ptr() as _, dock_tabs);
        igDockBuilderDockWindow(b"Properties\0".as_ptr() as _, dock_tabs);

        igDockBuilderFinish(dockspace_id);
    }
}

// Menu bar

fn draw_main_menu(ui: &Ui, state: &mut EditorState) {
    // begin_main_menu_bar returns Option<MainMenuBarToken>; the bar is active while token lives.
    if let Some(_bar) = ui.begin_main_menu_bar() {
        ui.menu("File", || {
            if ui.menu_item_with_shortcut("New", "Ctrl + N") {
                util::new_map(state);
            }
            ui.separator();
            if ui.menu_item_with_shortcut("Open…", "Ctrl + O") {
                util::open_map(state);
            }
            if ui.menu_item_with_shortcut("Save", "Ctrl + S") {
                util::save_map(state);
            }
            if ui.menu_item_with_shortcut("Save as…", "Ctrl + Shift + S") {
                util::save_map_as(state);
            }
            ui.separator();
            if ui.menu_item("Quit") {
                std::process::exit(0);
            }
        });

        ui.menu("View", || {
            let mut demo = state.show_demo;
            if ui.menu_item("ImGui demo") {
                demo = !demo;
            }
            state.show_demo = demo;

            ui.menu("Grid", || {
                let grid_steps = ["1", "2", "4", "8", "16", "32", "64", "128"];

                for (i, step) in grid_steps.iter().enumerate() {
                    let step_u8: u8 = step.parse().unwrap();
                    let mut selected = state.config.grid_minor_step == step_u8;
                    let active = !selected;
                    if ui.menu_item_toggle_with_shortcut(
                        step,
                        (i + 1).to_string(),
                        &mut selected,
                        active,
                    ) {
                        state
                            .config
                            .update("grid_minor_step", step_u8, &mut state.console);
                    }
                }
            });
        });

        ui.menu("Misc", || {
            ui.menu("Theme", || {
                for (i, entry) in state.themes.iter().enumerate() {
                    let mut selected = state.config.active_theme == i;
                    let active = !selected;
                    if ui.menu_item_toggle_no_shortcut(&entry.name, &mut selected, active) {
                        state.pending_theme = Some(i);
                    }
                }
            });
        });

        ui.menu("Help", || {
            if ui.menu_item("About Kradiant") {
                state.show_about = true;
            }
        });
    }

    const CTRL: u32 = 1;
    const SHIFT: u32 = 2;

    if !ui.io().want_text_input() {
        if key_combo_pressed(ui, dear_imgui_rs::Key::Z, CTRL) {
            let _ = state.undo.undo(
                &mut state.map,
                &mut state.selected_brushes,
                &mut state.selected_faces,
                &mut state.selected_entity,
                &mut state.map_revision,
                &mut state.console,
            );
        }
        if key_combo_pressed(ui, dear_imgui_rs::Key::Y, CTRL)
            || key_combo_pressed(ui, dear_imgui_rs::Key::Z, CTRL | SHIFT)
        {
            let _ = state.undo.redo(
                &mut state.map,
                &mut state.selected_brushes,
                &mut state.selected_faces,
                &mut state.selected_entity,
                &mut state.map_revision,
                &mut state.console,
            );
        }
    }

    if key_combo_pressed(ui, dear_imgui_rs::Key::N, CTRL) {
        util::new_map(state);
    }
    if key_combo_pressed(ui, dear_imgui_rs::Key::O, CTRL) {
        util::open_map(state);
    }
    if key_combo_pressed(ui, dear_imgui_rs::Key::S, CTRL) {
        util::save_map(state);
    }
    if key_combo_pressed(ui, dear_imgui_rs::Key::S, CTRL | SHIFT) {
        util::save_map_as(state);
    }
}

fn draw_toolbar(ui: &Ui, state: &mut EditorState) {
    let vp = unsafe { dear_imgui_rs::sys::igGetMainViewport().as_ref().unwrap() };
    let vp_pos = vp.WorkPos;
    let vp_size = vp.WorkSize;

    unsafe {
        dear_imgui_rs::sys::igSetNextWindowPos(
            dear_imgui_rs::sys::ImVec2 {
                x: vp_pos.x,
                y: vp_pos.y,
            },
            dear_imgui_rs::sys::ImGuiCond_Always as i32,
            dear_imgui_rs::sys::ImVec2 { x: 0.0, y: 0.0 },
        );
        dear_imgui_rs::sys::igSetNextWindowSize(
            dear_imgui_rs::sys::ImVec2 {
                x: vp_size.x,
                y: 0.0,
            }, // height = auto
            dear_imgui_rs::sys::ImGuiCond_Always as i32,
        );
        dear_imgui_rs::sys::igSetNextWindowBgAlpha(1.0);
    }

    let flags = WindowFlags::NO_DECORATION
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_SCROLL_WITH_MOUSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::from_bits_truncate(1 << 13); // NoBringToDisplayFront

    ui.window("##toolbar").flags(flags).build(|| {
        // File operations
        if icon_button(ui, "##open_map", state.icons.open, "Open Map") {
            util::open_map(state);
        }
        ui.same_line();
        if icon_button(ui, "##save_map", state.icons.save, "Save Map") {
            util::save_map(state);
        }

        ui.same_line();
        ui.separator_vertical();
        ui.same_line();
        if icon_button_toggle(
            ui,
            "##grid_snapping",
            state.icons.grid_snap,
            "Grid Snap",
            state.config.grid_snap,
            "Toggle Grid Snapping",
        ) {
            state.config.update(
                "grid_snap",
                !state.config.grid_snap as u8,
                &mut state.console,
            );
        }

        ui.same_line();

        if icon_button(ui, "##switch_view", state.icons.view_cycle, "Switch View") {
            let old_center = state.view2d.get_center();
            state.view2d.ortho_axis = state.view2d.ortho_axis.next();
            state.view2d.set_center(old_center);

            if let Some(aabb) = state.view2d.last_aabb.clone() {
                view2d::update_last_work_from_aabb(&mut state.view2d, &aabb);
            }
        }

        if ui.is_key_down(dear_imgui_rs::Key::LeftShift) && ui.is_key_pressed(dear_imgui_rs::Key::C)
        {
            if !state.selected_brushes.is_empty() {
                state.view2d.center_to_work();
                log_info!(state.console, "Goto Selection");
            }
        }

        ui.same_line();
        ui.separator_vertical();
        ui.same_line();

        let stretch_icon = match state.stretch_mode {
            StretchMode::Scale => state.icons.free_scale,
            StretchMode::Resize => state.icons.resize,
        };
        if icon_button(ui, "##stretch_mode", stretch_icon, "Stretch Mode") {
            state.stretch_mode = match state.stretch_mode {
                StretchMode::Scale => StretchMode::Resize,
                StretchMode::Resize => StretchMode::Scale,
            };
            ui.tooltip_text("Stretch behavior when dragging outside selection");
        }

        ui.same_line();

        if icon_button_toggle(
            ui,
            "##rotate_mode",
            state.icons.free_rotate,
            "Rotate",
            state.rotate_mode,
            "Free Rotation",
        ) {
            state.rotate_mode = !state.rotate_mode;
        }

        ui.same_line();
        ui.separator_vertical();
        ui.same_line();

        // Axis lock buttons
        if icon_button_toggle(
            ui,
            "##lock_x",
            state.icons.lock_x,
            "Lock X",
            state.axis_lock.x,
            "Lock all transformations on X-axis",
        ) {
            state.axis_lock.x = !state.axis_lock.x;
        }
        ui.same_line();
        if icon_button_toggle(
            ui,
            "##lock_y",
            state.icons.lock_y,
            "Lock Y",
            state.axis_lock.y,
            "Lock all transformations on Y-axis",
        ) {
            state.axis_lock.y = !state.axis_lock.y;
        }
        ui.same_line();
        if icon_button_toggle(
            ui,
            "##lock_z",
            state.icons.lock_z,
            "Lock Z",
            state.axis_lock.z,
            "Lock all transformations on Z-axis",
        ) {
            state.axis_lock.z = !state.axis_lock.z;
        }

        ui.same_line();
        ui.separator_vertical();
        ui.same_line();

        // Edit mode buttons
        if icon_button_toggle(
            ui,
            "##edit_faces",
            state.icons.edit_face,
            "Edit Faces",
            state.edit_faces,
            "Manipulate Faces",
        ) {
            state.edit_faces = !state.edit_faces;
            state.edit_edges = false;
            state.edit_vertices = false;
        }
        ui.same_line();
        if icon_button_toggle(
            ui,
            "##edit_edges",
            state.icons.edit_edge,
            "Edit Edges",
            state.edit_edges,
            "Manipulate Edges",
        ) {
            state.edit_edges = !state.edit_edges;
            state.edit_faces = false;
            state.edit_vertices = false;
        }
        ui.same_line();
        if icon_button_toggle(
            ui,
            "##edit_vertices",
            state.icons.edit_vertex,
            "Edit Vertices",
            state.edit_vertices,
            "Manipulate Vertices",
        ) {
            state.edit_vertices = !state.edit_vertices;
            state.edit_faces = false;
            state.edit_edges = false;
        }

        // Store the toolbar height so the dockspace can offset below it.
        state.toolbar_height = ui.window_size()[1];
    });
}

// Entity list

fn draw_entity_list(ui: &Ui, state: &mut EditorState) {
    ui.window("Entities")
        .size([220.0, 500.0], Condition::FirstUseEver)
        .build(|| {
            let Some(map) = &state.map else {
                ui.text_disabled("(no map loaded)");
                return;
            };
            for (i, ent) in map.entities.iter().enumerate() {
                let label = format!("{} ({})\0", ent.classname, ent.id.0);
                let selected = state.selected_entity == Some(i);
                if ui
                    .selectable_config(&label[..label.len() - 1])
                    .selected(selected)
                    .build()
                {
                    state.selected_entity = if selected { None } else { Some(i) };
                }
            }
        });
}

// Properties

fn draw_properties(ui: &Ui, state: &mut EditorState) {
    ui.window("Properties")
        .size([220.0, 280.0], Condition::FirstUseEver)
        .build(|| {
            let Some(idx) = state.selected_entity else {
                ui.text_disabled("(select an entity)");
                return;
            };
            let Some(map) = state.map.as_mut() else {
                ui.text_disabled("(select an entity)");
                return;
            };
            if idx >= map.entities.len() {
                ui.text_disabled("(select an entity)");
                return;
            }

            ui.text(format!("classname: {}", map.entities[idx].classname));
            ui.separator();

            let mut keys: Vec<String> = map.entities[idx].properties.keys().cloned().collect();
            keys.sort();

            let mut to_delete: Option<String> = None;

            for key in &keys {
                if ui.small_button(format!("-##{key}")) {
                    to_delete = Some(key.clone());
                }
                ui.same_line();
                ui.text(key);
                ui.same_line();
                ui.set_next_item_width(-1.0);
                let value = map.entities[idx].properties.get_mut(key).unwrap();
                ui.input_text(format!("##{key}"), value).build();
            }

            if let Some(k) = to_delete {
                state.undo.push_map(
                    "Delete property",
                    &*map,
                    &state.selected_brushes,
                    &state.selected_faces,
                    &state.selected_entity,
                );
                map.entities[idx].properties.remove(&k);
                state.map_revision = state.map_revision.wrapping_add(1);
            }

            ui.separator();

            // Add new property
            ui.set_next_item_width(ui.content_region_avail()[0] * 0.45);
            ui.input_text("##new_key", &mut state.new_prop_key)
                .hint("key")
                .build();
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_text("##new_val", &mut state.new_prop_val)
                .hint("value")
                .build();

            let can_add = !state.new_prop_key.trim().is_empty();
            /*if !can_add {
                 *                ui.push_style_color(StyleColor::Button,      [0.3, 0.3, 0.3, 1.0]);
                 *                ui.push_style_color(StyleColor::ButtonHovered,[0.3, 0.3, 0.3, 1.0]);
            }*/
            if ui.button("Add property") && can_add {
                state.undo.push_map(
                    "Add property",
                    &*map,
                    &state.selected_brushes,
                    &state.selected_faces,
                    &state.selected_entity,
                );
                map.entities[idx]
                    .properties
                    .entry(state.new_prop_key.trim().to_string())
                    .or_insert_with(|| state.new_prop_val.clone());
                state.new_prop_key.clear();
                state.new_prop_val.clear();
                state.map_revision = state.map_revision.wrapping_add(1);
            }
            /*if !can_add {
                 *                ui.pop_style_color();
                 *                ui.pop_style_color();
            }*/
        });
}

// Texture Browser

fn draw_texture_browser(ui: &Ui, state: &mut EditorState) {
    let mut on_select = |selected: String, map: &mut Option<Map>| {
        //log_info!(state.console, "selected: {selected}");
        let selected_brushes = state.selected_brushes.clone();
        let selected_faces = state.selected_faces.clone();
        let mut any = false;

        if state.edit_faces {
            if map.is_some() && !selected_faces.is_empty() {
                state.undo.push(
                    "Set face texture",
                    &*map,
                    &selected_brushes,
                    &selected_faces,
                    &state.selected_entity,
                );
            }

            let Some(mut_map) = map.as_mut() else {
                return;
            };
            for sel in selected_faces {
                let Some(entity) = mut_map.entities.get_mut(sel.entity_idx) else {
                    continue;
                };
                let Some(brush) = entity.brushes.get_mut(sel.brush_idx) else {
                    continue;
                };
                let kradiant::map::BrushContent::Convex(faces) = &mut brush.content else {
                    continue;
                };
                let Some(face) = faces.get_mut(sel.face_idx) else {
                    continue;
                };
                face.texture = selected.clone();
                any = true;
            }
        } else {
            if map.is_some() && !selected_brushes.is_empty() {
                state.undo.push(
                    "Set texture",
                    &*map,
                    &selected_brushes,
                    &selected_faces,
                    &state.selected_entity,
                );
            }

            let Some(mut_map) = map.as_mut() else {
                return;
            };
            for (entity_idx, brush_idx) in selected_brushes {
                let Some(entity) = mut_map.entities.get_mut(entity_idx) else {
                    continue;
                };
                let Some(brush) = entity.brushes.get_mut(brush_idx) else {
                    continue;
                };
                brush.apply_texture(&selected);
                any = true;
            }
        }

        if any {
            state.map_revision = state.map_revision.wrapping_add(1);
        }
    };

    texbro::draw_texture_browser(
        ui,
        &mut state.tex_browser,
        &mut state.tex_filter,
        &mut state.tex_tile_size,
        &mut on_select,
        &mut state.map,
    );
}

fn draw_about_dialog(ui: &Ui) {
    // title line — measure combined width first
    let title = "Kradiant Editor";
    let version = format!("v{}", crate::EDITOR_VERSION);
    let title_w = text_width(ui, title);
    let ver_w = text_width(ui, &version);
    let spacing = ui.clone_style().item_spacing()[0];
    util::center_next(ui, title_w + spacing + ver_w);
    ui.text_colored([0.95, 0.85, 0.3, 1.0], title);
    ui.same_line();
    ui.text_colored([0.6, 0.6, 0.6, 1.0], &version);

    // body text
    let body = "A modern map editor for CoD written in Rust";
    util::center_next(ui, text_width(ui, body));
    ui.text(body);

    ui.spacing();
    ui.separator();
    ui.spacing();

    let pb = "Powered by:";
    util::center_next(ui, text_width(ui, pb));
    ui.text(pb);
    let libs_ver = format!(
        "Dear ImGui v{}\nKradiant Library v{}",
        dear_imgui_rs::dear_imgui_version(),
        kradiant::KRADIANT_VERSION
    );
    util::center_next(ui, text_width(ui, &libs_ver));
    ui.text_colored([0.8, 0.8, 0.8, 1.0], libs_ver);

    ui.spacing();
    ui.separator();
    ui.spacing();

    let dn = "Donate";
    let gl = "GitLab";
    let sep = "|";
    let links_w = text_width(ui, dn) + text_width(ui, gl) + text_width(ui, sep) + 16.0;
    util::center_next(ui, links_w);
    ui.text_link_open_url(dn, "https://kazam.pages.dev/donate.html");
    ui.same_line();
    ui.text(sep);
    ui.same_line();
    ui.text_link_open_url(gl, "https://gitlab.com/kazam0180/kradiant");

    ui.spacing();
    ui.separator();
    ui.spacing();

    let cr1 = "© 2025 Kazam";
    util::center_next(ui, text_width(ui, cr1));
    ui.text_disabled(cr1);

    let cr2 = "This program comes with";
    util::center_next(ui, text_width(ui, cr2));
    ui.text_disabled(cr2);

    let cr3 = "absolutely no warranty.";
    util::center_next(ui, text_width(ui, cr3));
    ui.text_disabled(cr3);

    let lic = "GNU GPLv3";
    util::center_next(ui, text_width(ui, lic));
    ui.text_link_open_url(
        lic,
        "https://gitlab.com/kazam0180/kradiant/-/blob/main/LICENSE",
    );
}
