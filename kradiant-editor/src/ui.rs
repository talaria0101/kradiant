//! Editor UI
//!

use dear_imgui_rs::{Condition, StyleColor, TextureId, Ui, WindowFlags};

use crate::config::EditorConfig;
use crate::util::{project_to_2d, text_height};
use crate::{EDITOR_THEMES, util};
use glam::{Vec2, Vec3};
use kradiant::editing::{self, Aabb};
use kradiant::map::BrushId;
use kradiant::map_utils::format_float;
//use kradiant::loader::texture_loader;
use crate::icons::EditorIcons;
use crate::images::EditorImages;
use crate::theme::{EditorPalette, ThemeEntry, theme_from_str};
use util::{
    clamp_stretch_delta, normalize_depth, pack_abgr, project_aabb_to_2d, screen_to_world, snap,
    stretch_handle_point_2d, text_width,
};

// State types

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ortho {
    #[default]
    XY,
    XZ,
    YZ,
}

impl Ortho {
    fn label(self) -> &'static str {
        match self {
            Ortho::XY => "XY (top)",
            Ortho::XZ => "XZ (front)",
            Ortho::YZ => "YZ (side)",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::XY => Self::XZ,
            Self::XZ => Self::YZ,
            Self::YZ => Self::XY,
        }
    }
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum DragMode {
    #[default]
    NewBrush,
    MoveSelection,
    StretchSelection,
    RotateSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchMode {
    Scale,
    Resize,
}

impl Default for StretchMode {
    fn default() -> Self {
        Self::Scale
    }
}

#[derive(Clone)]
struct StretchDrag {
    selection_aabb: Aabb,
    faces: [Option<editing::StretchFace>; 2],
}

#[derive(Clone)]
struct RotateDrag {
    selection_aabb: Aabb,
    pivot_uv: [f32; 2],
    start_uv: [f32; 2],
}

pub struct LogEntry {
    pub level: LogLevel,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    fn prefix(self) -> &'static str {
        match self {
            LogLevel::Info => "   ",
            LogLevel::Warn => "[W]",
            LogLevel::Error => "[E]",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AxisLock {
    pub x: bool,
    pub y: bool,
    pub z: bool
}
/*
impl Default for AxisLock {
    fn default() -> Self
    {
        Self { x: false, y: false, z: false }
    }
}
*/
pub struct EditorState {
    pub config: EditorConfig,
    pub show_demo: bool,
    pub show_about: bool,
    pub toolbar_height: f32,
    pub map_path: String,
    pub grid_snapping: bool,
    pub ortho_axis: Ortho,
    pub work_pos: Vec3,
    pub work_depth: Vec3,
    pub view2d_rect: [f32; 4],
    pub view2d_zoom: f32,
    pub view2d_pan: [f32; 2],
    pub view2d_drag_start: Option<Vec2>,
    pub view2d_drag_current: Option<Vec2>,
    pub view2d_drag_mode: DragMode,
    pub view2d_tex_id: Option<TextureId>,
    /// Offset for rendering when we are moving something
    pub view2d_move_offset: Vec3,
    view2d_stretch: Option<StretchDrag>,
    view2d_stretch_delta: Vec3,
    view2d_rotate: Option<RotateDrag>,
    view2d_rotate_angle: f32,
    pub stretch_mode: StretchMode,
    pub rotate_mode: bool,
    pub axis_lock: AxisLock,
    pub selection_rgba: [f32; 4],
    pub tex_filter: String,
    pub tex_selected: Option<String>,
    pub tex_tile_size: f32,
    pub log: Vec<LogEntry>,
    //pub console_input:  String,
    pub console_scroll: bool,
    pub con_filter: String,
    pub map: Option<kradiant::map::Map>,
    pub selected_entity: Option<usize>,
    pub selected_brushes: Vec<(usize, usize)>,
    pub last_aabb: Option<Aabb>,
    pub new_prop_key: String,
    pub new_prop_val: String,
    pub icons: EditorIcons,
    pub images: EditorImages,
    pub themes: Vec<ThemeEntry>,
    pub pending_theme: Option<usize>,
    pub palette: EditorPalette,
}

/*macro_rules! editor_log {
    (info, $($arg:tt)+) => {
        self.log_info(format!($($arg)+))
    };
    (warn, $($arg:tt)+) => {
        self.log_warn(format!($($arg)+))
    };
    (error, $($arg:tt)+) => {
        self.log_error(format!($($arg)+))
    };
}*/

macro_rules! editor_log {
    ($state:expr, info, $($arg:tt)+) => {
        $state.log_info(format!($($arg)+))
    };
    ($state:expr, warn, $($arg:tt)+) => {
        $state.log_warn(format!($($arg)+))
    };
    ($state:expr, error, $($arg:tt)+) => {
        $state.log_error(format!($($arg)+))
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
            grid_snapping: true,
            ortho_axis: Ortho::default(),
            work_pos: Vec3::ZERO,
            work_depth: Vec3::ZERO,
            view2d_rect: [0.0; 4],
            view2d_zoom: 1.0,
            view2d_pan: [0.0, 0.0],
            view2d_drag_start: None,
            view2d_drag_current: None,
            view2d_drag_mode: DragMode::NewBrush,
            view2d_tex_id: None,
            view2d_move_offset: Vec3::ZERO,
            view2d_stretch: None,
            view2d_stretch_delta: Vec3::ZERO,
            view2d_rotate: None,
            view2d_rotate_angle: 0.0,
            stretch_mode: StretchMode::default(),
            rotate_mode: false,
            axis_lock: AxisLock::default(),
            selection_rgba: [0.3, 0.6, 1.0, 1.0],
            tex_filter: String::new(),
            tex_selected: None,
            tex_tile_size: 64.0,
            log: Vec::new(),
            //console_input:  String::new(),
            console_scroll: false,
            con_filter: String::new(),
            map: None,
            selected_entity: None,
            selected_brushes: Vec::new(),
            last_aabb: None,
            new_prop_key: String::new(),
            new_prop_val: String::new(),
            icons: EditorIcons::default(),
            images: EditorImages::default(),
            themes,
            pending_theme: None,
            palette: EditorPalette::default(),
        };
        s.log_info("Kradiant editor started");

        match EditorConfig::load() {
            Ok(c) => s.config = c,
            Err(e) => editor_log!(
                &mut s,
                error,
                "Failed to load configuration: {}",
                e.to_string()
            ),
        }

        s.log_warn("this is a warning");
        s
    }
}

impl EditorState {
    pub fn log_info(&mut self, msg: impl Into<String>) {
        let string: String = msg.into();
        println!("{}", &string);
        self.log.push(LogEntry {
            level: LogLevel::Info,
            text: string,
        });
        self.console_scroll = true;
    }
    pub fn log_warn(&mut self, msg: impl Into<String>) {
        let string: String = msg.into();
        println!("{}", &string);
        self.log.push(LogEntry {
            level: LogLevel::Warn,
            text: string,
        });
        self.console_scroll = true;
    }
    pub fn log_error(&mut self, msg: impl Into<String>) {
        let string: String = msg.into();
        eprintln!("{}", &string);
        self.log.push(LogEntry {
            level: LogLevel::Error,
            text: string,
        });
        self.console_scroll = true;
    }
    /*
        pub(crate) fn view2d_preview_point(&self, p: Vec3) -> Vec3
        {
            match self.view2d_drag_mode {
                DragMode::MoveSelection => p + self.view2d_move_offset.as_vec3(),
                DragMode::StretchSelection => {
                    let Some(stretch) = self.view2d_stretch.as_ref() else {
                        return p;
                    };
                    let Some((xform, _preview)) = editing::stretch_selection_transform(
                        &stretch.selection_aabb,
                        stretch.faces,
                        self.view2d_stretch_delta,
                    ) else {
                        return p;
                    };
                    xform.apply_point(p)
                }
                DragMode::NewBrush => p,
            }
        }
    */
    pub(crate) fn view2d_stretch_preview_xform(&self) -> Option<editing::AffineScale> {
        let stretch = self.view2d_stretch.as_ref()?;
        editing::stretch_selection_transform(
            &stretch.selection_aabb,
            stretch.faces,
            self.view2d_stretch_delta,
        )
        .map(|(xform, _)| xform)
    }

    pub(crate) fn view2d_face_stretch_preview(
        &self,
    ) -> Option<([Option<editing::StretchFace>; 2], Vec3)> {
        let stretch = self.view2d_stretch.as_ref()?;
        Some((stretch.faces, self.view2d_stretch_delta))
    }

    pub(crate) fn view2d_rotate_preview_xform(&self) -> Option<editing::AffineRotate> {
        let rotate = self.view2d_rotate.as_ref()?;
        let axis = match self.ortho_axis {
            Ortho::XY => Vec3::Z,
            Ortho::XZ => Vec3::Y,
            Ortho::YZ => Vec3::X,
        };
        editing::rotate_selection_transform(&rotate.selection_aabb, axis, self.view2d_rotate_angle)
            .map(|(xform, _)| xform)
    }

    pub fn is_rotation_locked(&self) -> bool {
        match self.ortho_axis {
            Ortho::XY => self.axis_lock.y,
            Ortho::XZ | Ortho::YZ => self.axis_lock.z,
        }
    }

    pub fn view2d_center_world(&self) -> Vec3 {
        let zoom = self.view2d_zoom.max(0.001);
        let world_x = -self.view2d_pan[0] / zoom;
        let world_y = -self.view2d_pan[1] / zoom;

        match self.ortho_axis {
            Ortho::XY => Vec3::new(world_x, world_y, self.work_pos.z),
            Ortho::XZ => Vec3::new(world_x, self.work_pos.y, -world_y),
            Ortho::YZ => Vec3::new(self.work_pos.x, world_x, -world_y),
        }
    }

    pub fn set_view2d_center(&mut self, world_center: Vec3) {
        let zoom = self.view2d_zoom.max(0.001);

        let (target_x, target_y) = match self.ortho_axis {
            Ortho::XY => (world_center.x, world_center.y),
            Ortho::XZ => (world_center.x, -world_center.z),
            Ortho::YZ => (world_center.y, -world_center.z),
        };

        self.view2d_pan[0] = -target_x * zoom;
        self.view2d_pan[1] = -target_y * zoom;
    }
}

// Top-level draw call

pub fn draw_editor(ui: &Ui, state: &mut EditorState, dt: f32) {
    draw_dockspace(ui, state);
    draw_main_menu(ui, state);
    draw_toolbar(ui, state);
    draw_entity_list(ui, state);
    draw_properties(ui, state);
    draw_view3d(ui, state);
    draw_view2d(ui, state, dt);
    draw_console(ui, state);
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
                        //state.config.grid_minor_step = step_u8;
                        EditorConfig::update(state, "grid_minor_step", step_u8);
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

    if ui.is_key_down(dear_imgui_rs::Key::LeftCtrl) && ui.is_key_pressed(dear_imgui_rs::Key::N) {
        util::new_map(state);
    }
    if ui.is_key_down(dear_imgui_rs::Key::LeftCtrl) && ui.is_key_pressed(dear_imgui_rs::Key::O) {
        util::open_map(state);
    }
    if ui.is_key_down(dear_imgui_rs::Key::LeftCtrl) && ui.is_key_pressed(dear_imgui_rs::Key::S) {
        util::save_map(state);
    }
    if ui.is_key_down(dear_imgui_rs::Key::LeftCtrl)
        && ui.is_key_down(dear_imgui_rs::Key::LeftShift)
        && ui.is_key_pressed(dear_imgui_rs::Key::S)
    {
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
        let open_map = if let Some(tid) = state.icons.open {
            // image_button(id, texture_id, size) — the str id disambiguates multiple image buttons
            //ui.image_button("##switch_view", tid, [24.0, 24.0])
            ui.image_button_config("##open_map", tid, [24.0, 24.0])
                .build()
        } else {
            ui.small_button("Open") // fallback if texture didn't load
        };
        if open_map {
            util::open_map(state);
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Open Map");
        }

        ui.same_line();

        let save_map = if let Some(tid) = state.icons.save {
            // image_button(id, texture_id, size) — the str id disambiguates multiple image buttons
            //ui.image_button("##switch_view", tid, [24.0, 24.0])
            ui.image_button_config("##save_map", tid, [24.0, 24.0])
                .build()
        } else {
            ui.small_button("Open") // fallback if texture didn't load
        };
        if save_map {
            util::save_map(state);
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Save Map");
        }

        ui.same_line();
        ui.separator_vertical(); // vertical separator
        ui.same_line();

        let toggle_snap = if let Some(tid) = state.icons.grid_snap {
            let tint_col = match state.grid_snapping {
                true => [1.0, 1.0, 1.0, 1.0],
                false => [1.0, 1.0, 1.0, 0.5],
            };
            ui.image_button_config("##grid_snapping", tid, [24.0, 24.0])
            .tint_color(tint_col)
            .build()
        } else {
            ui.button("Grid Snap")
        };
        if toggle_snap {
            state.grid_snapping = !state.grid_snapping;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Toggle Grid Snapping");
        }

        ui.same_line();

        let view_switched = if let Some(tid) = state.icons.view_cycle {
            ui.image_button_config("##switch_view", tid, [24.0, 24.0])
            .build()
        } else {
            ui.small_button("Switch") // fallback if texture didn't load
        };
        if view_switched {
            let old_center = state.view2d_center_world();
            state.ortho_axis = state.ortho_axis.next();
            state.set_view2d_center(old_center);

            if let Some(aabb) = state.last_aabb.clone() {
                update_last_work_from_aabb(state, &aabb);
            }
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Switch View");
        }

        if ui.is_key_down(dear_imgui_rs::Key::LeftShift) && ui.is_key_pressed(dear_imgui_rs::Key::C) {
            if !state.selected_brushes.is_empty() {
                state.set_view2d_center(state.work_pos);
                editor_log!(state, info, "Goto Selection");
            }
        }

        ui.same_line();
        ui.separator_vertical();
        ui.same_line();

        let stretch_icon = match state.stretch_mode {
            StretchMode::Scale => state.icons.free_scale,
            StretchMode::Resize => state.icons.resize,
        };
        if ui
            .image_button_config(
                &format!("##stretch_mode"),
                stretch_icon.unwrap(),
                [24.0, 24.0],
            )
            .build()
        {
            state.stretch_mode = match state.stretch_mode {
                StretchMode::Scale => StretchMode::Resize,
                StretchMode::Resize => StretchMode::Scale,
            };
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Stretch behavior when dragging outside selection");
        }

        ui.same_line();

        let toggle_rotate = if let Some(tid) = state.icons.free_rotate {
            let tint_col = match state.rotate_mode {
                true => [1.0, 1.0, 1.0, 1.0],
                false => [1.0, 1.0, 1.0, 0.5],
            };
            ui.image_button_config("##rotate_mode", tid, [24.0, 24.0])
                .tint_color(tint_col)
                .build()
        } else {
            ui.button("Rotate")
        };
        if toggle_rotate {
            state.rotate_mode = !state.rotate_mode;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Free Rotation");
        }

        ui.same_line();
        ui.separator_vertical();
        ui.same_line();

        let lock_x = if let Some(tid) = state.icons.lock_x {
            let tint_col = match state.axis_lock.x {
                true => [1.0, 1.0, 1.0, 1.0],
                false => [1.0, 1.0, 1.0, 0.5],
            };
            ui.image_button_config("##lock_x", tid, [24.0, 24.0])
            .tint_color(tint_col)
            .build()
        } else {
            ui.button("Lock X")
        };
        if lock_x {
            state.axis_lock.x = !state.axis_lock.x;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Lock all transformations on X-axis");
        }

        ui.same_line();

        let lock_y = if let Some(tid) = state.icons.lock_y {
            let tint_col = match state.axis_lock.y {
                true => [1.0, 1.0, 1.0, 1.0],
                false => [1.0, 1.0, 1.0, 0.5],
            };
            ui.image_button_config("##lock_y", tid, [24.0, 24.0])
            .tint_color(tint_col)
            .build()
        } else {
            ui.button("Lock Y")
        };
        if lock_y {
            state.axis_lock.y = !state.axis_lock.y;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Lock all transformations on Y-axis");
        }

        ui.same_line();

        let lock_z = if let Some(tid) = state.icons.lock_z {
            let tint_col = match state.axis_lock.z {
                true => [1.0, 1.0, 1.0, 1.0],
                false => [1.0, 1.0, 1.0, 0.5],
            };
            ui.image_button_config("##lock_z", tid, [24.0, 24.0])
            .tint_color(tint_col)
            .build()
        } else {
            ui.button("Lock Z")
        };
        if lock_z {
            state.axis_lock.z = !state.axis_lock.z;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Lock all transformations on Z-axis");
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
            let (Some(map), Some(idx)) = (&mut state.map, state.selected_entity) else {
                ui.text_disabled("(select an entity)");
                return;
            };
            let ent = &mut map.entities[idx];

            ui.text(format!("classname: {}", ent.classname));
            ui.separator();

            let mut keys: Vec<String> = ent.properties.keys().cloned().collect();
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
                let value = ent.properties.get_mut(key).unwrap();
                ui.input_text(format!("##{key}"), value).build();
            }

            if let Some(k) = to_delete {
                ent.properties.remove(&k);
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
                ui.push_style_color(StyleColor::Button,      [0.3, 0.3, 0.3, 1.0]);
                ui.push_style_color(StyleColor::ButtonHovered,[0.3, 0.3, 0.3, 1.0]);
            }*/
            if ui.button("Add property") && can_add {
                ent.properties
                    .entry(state.new_prop_key.trim().to_string())
                    .or_insert_with(|| state.new_prop_val.clone());
                state.new_prop_key.clear();
                state.new_prop_val.clear();
            }
            /*if !can_add {
                ui.pop_style_color();
                ui.pop_style_color();
            }*/
        });
}

// 3D View

fn draw_view3d(ui: &Ui, state: &mut EditorState) {
    ui.window("3D View")
        .size([640.0, 480.0], Condition::FirstUseEver)
        .build(|| {
            ui.text("FOV");
            ui.same_line();
            ui.set_next_item_width(80.0);
            ui.slider_config("##fov", 40.0f32, 120.0f32)
                .display_format("%.0f°")
                .build(&mut state.config.view3d_fov);

            ui.separator();

            let [w, h] = ui.content_region_avail();
            let (w, h) = (w.max(1.0), h.max(1.0));
            let p = ui.cursor_screen_pos();

            /*let draw = ui.get_window_draw_list();
            draw.add_rect(p, [p[0] + w, p[1] + h], 0xFF20_2020u32).filled(true).build();
            draw.add_rect(p, [p[0] + w, p[1] + h], 0xFF44_4444u32).filled(false).build();*/

            let label = "3D View";
            let lw = text_width(ui, label);
            ui.set_cursor_screen_pos([p[0] + (w - lw) * 0.5, p[1] + h * 0.5 - 7.0]);
            ui.text_disabled(label);

            ui.set_cursor_screen_pos(p);
            ui.invisible_button("##3d_hit", [w, h]);
            // TODO: camera mouse-look / WASD when item is active/hovered
        });
}

// 2D View

fn draw_view2d(ui: &Ui, state: &mut EditorState, dt: f32) {
    ui.window("2D View")
        .size([640.0, 480.0], Condition::FirstUseEver)
        .flags(WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE)
        .build(|| {
            /*if ui.small_button("Switch") {
                state.ortho_axis = state.ortho_axis.next();
                if let Some(aabb) = state.last_aabb.clone() {
                    update_last_work_from_aabb(state, &aabb);
                }
            }*/

            let [w, h] = ui.content_region_avail();
            let (w, h) = (w.max(1.0), h.max(1.0));
            let p = ui.cursor_screen_pos();
            let draw = ui.get_window_draw_list();
            state.selection_rgba = ui.style_color(StyleColor::ButtonActive);

            state.view2d_rect = [p[0], p[1], w, h];

            ui.set_cursor_screen_pos(p);
            ui.invisible_button("##2d_canvas", [w, h]);
            let canvas_interacting = ui.is_item_hovered() || ui.is_item_active();

            if canvas_interacting {
                let wheel = ui.io().mouse_wheel();
                if wheel != 0.0 {
                    let mouse = ui.io().mouse_pos();
                    let old_zoom = state.view2d_zoom;
                    let f = if wheel > 0.0 { 1.15f32 } else { 1.0 / 1.15 };
                    let new_zoom = (old_zoom * f).clamp(0.025, 64.0);
                    if (new_zoom - old_zoom).abs() > f32::EPSILON {
                        let world = screen_to_world(mouse, p, [w, h], old_zoom, state.view2d_pan);
                        state.view2d_zoom = new_zoom;
                        state.view2d_pan[0] = (mouse[0] - (p[0] + w * 0.5)) - world[0] * new_zoom;
                        state.view2d_pan[1] = (mouse[1] - (p[1] + h * 0.5)) - world[1] * new_zoom;
                    }
                }
                if ui.is_mouse_dragging(dear_imgui_rs::MouseButton::Right) {
                    let [dx, dy] = ui.mouse_drag_delta(dear_imgui_rs::MouseButton::Right);
                    state.view2d_pan[0] += dx;
                    state.view2d_pan[1] += dy;
                    ui.reset_mouse_drag_delta(dear_imgui_rs::MouseButton::Right);
                }
            }

            let mouse = ui.io().mouse_pos();
            let world = screen_to_world(mouse, p, [w, h], state.view2d_zoom, state.view2d_pan);
            let world_axis = match state.ortho_axis {
                Ortho::XY => world,
                Ortho::XZ | Ortho::YZ => [world[0], -world[1]],
            };

            let step = state.config.grid_minor_step as f32;
            let my_snapping = if ui.is_key_down(dear_imgui_rs::Key::LeftCtrl) {
                !state.grid_snapping
            } else { state.grid_snapping };
            // `snap()` returns f32; due to float error this can land just below an integer
            // (e.g. 23.999998) and truncating would break grid-step alignment.
            let (snapped, snapped_i) = if my_snapping {
                let snapped = [snap(world[0], step), snap(world[1], step)];
                let snapped_i = Vec2::new(snapped[0].round(), snapped[1].round());

                (snapped, snapped_i)
            }
            else {
                (world, world.into())
            };

            if canvas_interacting
                && (ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left)
                    || ui.is_mouse_dragging(dear_imgui_rs::MouseButton::Left))
                && ui.is_key_down(dear_imgui_rs::Key::LeftShift)
            {
                let ray_far = 1.0e6;
                let (ray_origin, ray_dir) = match state.ortho_axis {
                    Ortho::XY => (
                        Vec3::new(world[0], world[1], ray_far),
                        Vec3::new(0.0, 0.0, -1.0),
                    ),
                    Ortho::XZ => (
                        Vec3::new(world[0], ray_far, -world[1]),
                        Vec3::new(0.0, -1.0, 0.0),
                    ),
                    Ortho::YZ => (
                        Vec3::new(ray_far, world[0], -world[1]),
                        Vec3::new(-1.0, 0.0, 0.0),
                    ),
                };

                let selected_brush = state.map.as_mut().and_then(|m| {
                    editing::pick_brush_by_ray(m, ray_origin, ray_dir, editing::PickMask::ALL)
                });
                if let Some(sel) = selected_brush {
                    if !state.selected_brushes.contains(&sel) {
                        state.selected_brushes.push(sel);
                    }
                    state.selected_entity = Some(sel.0);
                    if let Some(aabb) = selection_aabb(state) {
                        state.last_aabb = Some(aabb.clone());
                        update_last_work_from_aabb(state, &aabb);
                    }
                }
            }

            let mut snapped_marker: Option<([f32; 2], u32)> = None;
            if canvas_interacting && ui.is_mouse_down(dear_imgui_rs::MouseButton::Left) {
                let sx = p[0] + w * 0.5 + state.view2d_pan[0] + snapped[0] * state.view2d_zoom;
                let sy = p[1] + h * 0.5 + state.view2d_pan[1] + snapped[1] * state.view2d_zoom;
                let col = util::imgui_color_to_u32(ui.style_color(StyleColor::TabSelectedOverline));
                snapped_marker = Some(([sx, sy], col));
            }
            // START drag
            if canvas_interacting
                && ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left)
                && !ui.is_key_down(dear_imgui_rs::Key::LeftShift)
            {
                state.view2d_move_offset = Vec3::ZERO;
                state.view2d_stretch = None;
                state.view2d_stretch_delta = Vec3::ZERO;
                state.view2d_rotate = None;
                state.view2d_rotate_angle = 0.0;
                state.view2d_drag_mode = if !state.selected_brushes.is_empty()
                {
                    if util::click_in_selection_aabb(state, snapped_i) {
                        if state.rotate_mode {
                            if let Some(aabb) = selection_aabb(state) {
                                let center =
                                    (aabb.min + aabb.max) * 0.5;
                                let pivot_uv = project_to_2d(center, state.ortho_axis);
                                state.view2d_rotate = Some(RotateDrag {
                                    selection_aabb: aabb,
                                    pivot_uv,
                                    start_uv: world,
                                });
                                DragMode::RotateSelection
                            } else {
                                DragMode::MoveSelection
                            }
                        } else {
                            DragMode::MoveSelection
                        }
                    } else if let Some(aabb) = selection_aabb(state) {
                        let faces = stretch_faces_from_start(state.ortho_axis, &aabb, snapped_i);
                        state.view2d_stretch = Some(StretchDrag {
                            selection_aabb: aabb,
                            faces: [faces.get(0).copied(), faces.get(1).copied()],
                        });
                        state.view2d_stretch_delta = Vec3::ZERO;
                        DragMode::StretchSelection
                    } else {
                        DragMode::NewBrush
                    }
                } else {
                    DragMode::NewBrush
                };
                state.view2d_drag_start = Some(snapped_i);
                state.view2d_drag_current = Some(snapped_i);
            }

            // UPDATE drag
            if canvas_interacting
                && ui.is_mouse_down(dear_imgui_rs::MouseButton::Left)
                && !ui.is_key_down(dear_imgui_rs::Key::LeftShift)
            // LShift + LMouse for selecting only
            {
                if state.view2d_drag_start.is_some() {
                    state.view2d_drag_current = Some(snapped_i);

                    if state.view2d_drag_mode == DragMode::MoveSelection {
                        let d = snapped_i - state.view2d_drag_start.unwrap();
                        state.view2d_move_offset = util::drag_delta_to_3d(d, state.ortho_axis, state.axis_lock);
                    } else if state.view2d_drag_mode == DragMode::StretchSelection {
                        let d = snapped_i - state.view2d_drag_start.unwrap();
                        let mut delta = util::drag_delta_to_3d(d, state.ortho_axis, state.axis_lock);
                        if let Some(stretch) = state.view2d_stretch.as_ref() {
                            delta = clamp_stretch_delta(
                                &stretch.selection_aabb,
                                stretch.faces,
                                delta,
                                state.config.grid_minor_step as i32,
                                my_snapping,
                            );
                        }
                        state.view2d_stretch_delta = delta;
                    } else if state.view2d_drag_mode == DragMode::RotateSelection {
                        if let Some(rot) = state.view2d_rotate.as_ref() {
                            let angle = if state.is_rotation_locked() {
                                0.0
                            } else {
                                let v0 = [
                                    rot.start_uv[0] - rot.pivot_uv[0], rot.start_uv[1] - rot.pivot_uv[1],
                                ];
                                let v1 = [world[0] - rot.pivot_uv[0], world[1] - rot.pivot_uv[1]];
                                let dot = v0[0] * v1[0] + v0[1] * v1[1];
                                let cross = v0[0] * v1[1] - v0[1] * v1[0];
                                let mut angle = cross.atan2(dot);
                                // `world` is in "screen" coordinates (V grows down). For XY/YZ this
                                // flips handedness vs. the 3D right-handed axis we rotate about.
                                if matches!(state.ortho_axis, Ortho::XY | Ortho::YZ) {
                                    angle = -angle;
                                }
                                if my_snapping {
                                    angle = angle.to_degrees().round().to_radians();
                                }

                                angle
                            };
                            state.view2d_rotate_angle = angle;
                        }
                    }

                    let edge_zone = 20.0;
                    let pan_speed = 100.0 * dt;

                    let mouse = ui.mouse_pos();
                    let [rx, ry, rw, rh] = state.view2d_rect;

                    if mouse[0] < rx + edge_zone {
                        state.view2d_pan[0] += pan_speed;
                    }
                    if mouse[0] > rx + rw - edge_zone {
                        state.view2d_pan[0] -= pan_speed;
                    }
                    if mouse[1] < ry + edge_zone {
                        state.view2d_pan[1] += pan_speed;
                    }
                    if mouse[1] > ry + rh - edge_zone {
                        state.view2d_pan[1] -= pan_speed;
                    }
                }
            }

            // FINISH drag
            if canvas_interacting && ui.is_mouse_released(dear_imgui_rs::MouseButton::Left) {
                if let (Some(start), Some(end)) =
                    (state.view2d_drag_start, state.view2d_drag_current)
                {
                    match state.view2d_drag_mode {
                        DragMode::MoveSelection => {
                            let d = end - start;
                            if d != Vec2::ZERO {
                                let delta = util::drag_delta_to_3d(d, state.ortho_axis, state.axis_lock);
                                if let Some(map) = state.map.as_mut() {
                                    let gen_ = &mut map.generation;
                                    for &(entity_idx, brush_idx) in &state.selected_brushes {
                                        if let Some(entity) = map.entities.get_mut(entity_idx) {
                                            if let Some(brush) = entity.brushes.get_mut(brush_idx) {
                                                brush.translate(gen_, delta);
                                            }
                                        }
                                    }
                                }
                                // Keep last_aabb in sync
                                if let Some(aabb) = selection_aabb(state) {
                                    state.last_aabb = Some(aabb.clone());
                                    update_last_work_from_aabb(state, &aabb);
                                }
                            }

                            state.view2d_move_offset = Vec3::ZERO;
                            editor_log!(state, info, "Dragged selection");
                        }
                        DragMode::NewBrush => {
                            if state.selected_brushes.is_empty() {
                                if let Some(created) = create_brush_from_drag(state, start, end) {
                                    let diff = start - end;
                                    let diff = (diff * diff).sqrt();
                                    if diff.x < 1.0 || diff.y < 1.0 {
                                        editor_log!(state, warn, "Brush planes smaller than `1` not recommended");
                                    }
                                    state.selected_brushes.push((0, created.0 as usize));
                                    if let Some(aabb) = selection_aabb(state) {
                                        state.last_aabb = Some(aabb.clone());
                                        update_last_work_from_aabb(state, &aabb);
                                    }
                                }
                            }
                        }
                        DragMode::StretchSelection => {
                            if let Some(stretch) = state.view2d_stretch.take() {
                                let delta = state.view2d_stretch_delta;
                                if delta != Vec3::ZERO {
                                    let mut new_sel_aabb: Option<Aabb> = None;
                                    if let Some(map) = state.map.as_mut() {
                                        let mut any = false;
                                        match state.stretch_mode {
                                            StretchMode::Scale => {
                                                if let Some((xform, _preview)) =
                                                    editing::stretch_selection_transform(
                                                        &stretch.selection_aabb,
                                                        stretch.faces,
                                                        delta,
                                                    )
                                                {
                                                    for &(entity_idx, brush_idx) in
                                                        &state.selected_brushes
                                                    {
                                                        let Some(entity) =
                                                            map.entities.get_mut(entity_idx)
                                                        else {
                                                            continue;
                                                        };
                                                        let Some(brush) =
                                                            entity.brushes.get_mut(brush_idx)
                                                        else {
                                                            continue;
                                                        };
                                                        if editing::apply_affine_scale_to_brush(
                                                            brush,
                                                            &mut map.generation,
                                                            xform,
                                                        ) {
                                                            any = true;
                                                        }
                                                    }
                                                }
                                            }
                                            StretchMode::Resize => {
                                                let xform_for_patches =
                                                    editing::stretch_selection_transform(
                                                        &stretch.selection_aabb,
                                                        stretch.faces,
                                                        delta,
                                                    )
                                                    .map(|(x, _)| x);

                                                for &(entity_idx, brush_idx) in
                                                    &state.selected_brushes
                                                {
                                                    let Some(entity) =
                                                        map.entities.get_mut(entity_idx)
                                                    else {
                                                        continue;
                                                    };
                                                    let Some(brush) =
                                                        entity.brushes.get_mut(brush_idx)
                                                    else {
                                                        continue;
                                                    };
                                                    match &brush.content {
                                                        kradiant::map::BrushContent::Convex(_) => {
                                                            if editing::stretch_convex_brush_faces(
                                                                brush,
                                                                &mut map.generation,
                                                                stretch.faces,
                                                                delta,
                                                                state.config.grid_minor_step as i32,
                                                            ) {
                                                                any = true;
                                                            }
                                                        }
                                                        kradiant::map::BrushContent::Patch(_) => {
                                                            if let Some(xform) = xform_for_patches
                                                            {
                                                                if editing::apply_affine_scale_to_brush(
                                                                    brush,
                                                                    &mut map.generation,
                                                                    xform,
                                                                ) {
                                                                    any = true;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        if any {
                                            new_sel_aabb = selection_aabb_from_map(
                                                map,
                                                &state.selected_brushes,
                                            );
                                            editor_log!(state, info, "Stretched selection");
                                        }
                                    }
                                    if let Some(aabb) = new_sel_aabb {
                                        state.last_aabb = Some(aabb.clone());
                                        update_last_work_from_aabb(state, &aabb);
                                    }
                                }
                            }
                            state.view2d_stretch_delta = Vec3::ZERO;
                        }
                        DragMode::RotateSelection => {
                            if let Some(rot) = state.view2d_rotate.take() {

                                let angle = if state.is_rotation_locked() {
                                    0.0
                                } else { state.view2d_rotate_angle };

                                if angle.abs() > 1.0e-6 {
                                    let axis = match state.ortho_axis {
                                        Ortho::XY => Vec3::Z,
                                        Ortho::XZ => Vec3::Y,
                                        Ortho::YZ => Vec3::X,
                                    };
                                    if let Some((xform, _preview)) = editing::rotate_selection_transform(
                                        &rot.selection_aabb,
                                        axis,
                                        angle,
                                    ) {
                                        let mut new_sel_aabb: Option<Aabb> = None;
                                        if let Some(map) = state.map.as_mut() {
                                            let mut any = false;
                                            for &(entity_idx, brush_idx) in &state.selected_brushes {
                                                let Some(entity) = map.entities.get_mut(entity_idx) else {
                                                    continue;
                                                };
                                                let Some(brush) = entity.brushes.get_mut(brush_idx) else {
                                                    continue;
                                                };
                                                if editing::apply_affine_rotate_to_brush(
                                                    brush,
                                                    &mut map.generation,
                                                    xform,
                                                ) {
                                                    any = true;
                                                }
                                            }

                                            if any {
                                                new_sel_aabb = selection_aabb_from_map(
                                                    map,
                                                    &state.selected_brushes,
                                                );
                                                editor_log!(state, info, "Rotated selection");
                                            }
                                        }
                                        if let Some(aabb) = new_sel_aabb {
                                            state.last_aabb = Some(aabb.clone());
                                            update_last_work_from_aabb(state, &aabb);
                                        }
                                    }
                                }
                            }
                            state.view2d_rotate_angle = 0.0;
                        }
                    }
                }
                state.view2d_drag_start = None;
                state.view2d_drag_current = None;
                state.view2d_stretch = None;
                state.view2d_stretch_delta = Vec3::ZERO;
                state.view2d_rotate = None;
                state.view2d_rotate_angle = 0.0;
            }

            if ui.is_key_pressed(dear_imgui_rs::Key::Escape) {
                if let Some(aabb) = selection_aabb(state) {
                    state.last_aabb = Some(aabb.clone());
                    update_last_work_from_aabb(state, &aabb);
                }
                state.selected_brushes.clear();
                state.view2d_stretch = None;
                state.view2d_stretch_delta = Vec3::ZERO;
                state.view2d_rotate = None;
                state.view2d_rotate_angle = 0.0;
                state.view2d_move_offset = Vec3::ZERO;
            }

            if ui.is_key_pressed(dear_imgui_rs::Key::Backspace) {
                if let Some(aabb) = selection_aabb(state) {
                    state.last_aabb = Some(aabb.clone());
                    update_last_work_from_aabb(state, &aabb);
                }

                if let Some(map) = state.map.as_mut() {
                    use std::collections::BTreeMap;
                    let mut by_entity: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
                    for &(entity_idx, brush_idx) in &state.selected_brushes {
                        by_entity.entry(entity_idx).or_default().push(brush_idx);
                    }

                    for brush_indices in by_entity.values_mut() {
                        brush_indices.sort_unstable();
                        brush_indices.dedup();
                        brush_indices.sort_unstable_by(|a, b| b.cmp(a));
                    }

                    for (entity_idx, brush_indices) in by_entity {
                        let Some(entity) = map.entities.get_mut(entity_idx) else {
                            continue;
                        };
                        for brush_idx in brush_indices {
                            if brush_idx < entity.brushes.len() {
                                entity.brushes.swap_remove(brush_idx);
                            }
                        }
                    }

                    map.generation = map.generation.wrapping_add(1);
                }

                state.selected_brushes.clear();
                state.selected_entity = None;
            }

            draw.with_clip_rect(p, [p[0] + w, p[1] + h], || {
                draw.add_rect(
                    p,
                    [p[0] + w, p[1] + h],
                    util::imgui_color_to_u32(state.palette.view2d_bg),
                )
                .filled(true)
                .build();

                if let Some(tid) = state.view2d_tex_id {
                    draw.add_image(
                        tid,
                        p,
                        [p[0] + w, p[1] + h],
                        [0.0, 1.0], // flip Y: OpenGL origin is bottom-left
                        [1.0, 0.0],
                        0xFFFFFFFFu32,
                    );
                }

                draw.add_text(
                    [p[0] + 8.0, p[1] + 6.0],
                    util::imgui_color_to_u32(state.palette.hud_text),
                    state.ortho_axis.label(),
                );
                if canvas_interacting {
                    draw.add_text(
                        [p[0] + 8.0, p[1] + 24.0],
                        util::imgui_color_to_u32(state.palette.hud_text_dim),
                        format!("{:.1}, {:.1}", world_axis[0], world_axis[1]),
                    );
                }

                draw.add_text(
                    [p[0] + 100.0, p[1] + 6.0],
                    util::imgui_color_to_u32(state.palette.hud_text_dim),
                    format!("{:.2} FPS (average)", ui.io().framerate()),
                );

                if ui.is_window_hovered() {
                    if ui.is_key_pressed(dear_imgui_rs::Key::Key1) {
                        EditorConfig::update(state, "grid_minor_step", 1);
                    }
                    if ui.is_key_pressed(dear_imgui_rs::Key::Key2) {
                        EditorConfig::update(state, "grid_minor_step", 2);
                    }
                    if ui.is_key_pressed(dear_imgui_rs::Key::Key3) {
                        EditorConfig::update(state, "grid_minor_step", 4);
                    }
                    if ui.is_key_pressed(dear_imgui_rs::Key::Key4) {
                        EditorConfig::update(state, "grid_minor_step", 8);
                    }
                    if ui.is_key_pressed(dear_imgui_rs::Key::Key5) {
                        EditorConfig::update(state, "grid_minor_step", 16);
                    }
                    if ui.is_key_pressed(dear_imgui_rs::Key::Key6) {
                        EditorConfig::update(state, "grid_minor_step", 32);
                    }
                    if ui.is_key_pressed(dear_imgui_rs::Key::Key7) {
                        EditorConfig::update(state, "grid_minor_step", 64);
                    }
                    if ui.is_key_pressed(dear_imgui_rs::Key::Key8) {
                        EditorConfig::update(state, "grid_minor_step", 128);
                    }
                }

                if let Some((pos, col)) = snapped_marker {
                    draw.add_circle(pos, 4.0, col).filled(true).build();
                }

                if let (Some(start), Some(end)) =
                    (state.view2d_drag_start, state.view2d_drag_current)
                {
                    let col = ui.style_color(StyleColor::TabSelectedOverline);

                    let to_screen = |v: Vec2| -> [f32; 2] {
                        [
                            p[0] + w * 0.5 + state.view2d_pan[0] + v.x * state.view2d_zoom,
                            p[1] + h * 0.5 + state.view2d_pan[1] + v.y * state.view2d_zoom,
                        ]
                    };
                    let to_screen_f = |v: [f32; 2]| -> [f32; 2] {
                        [
                            p[0] + w * 0.5 + state.view2d_pan[0] + v[0] * state.view2d_zoom,
                            p[1] + h * 0.5 + state.view2d_pan[1] + v[1] * state.view2d_zoom,
                        ]
                    };

                    match state.view2d_drag_mode {
                        DragMode::MoveSelection => {
                            let (a, b) = if let Some(sel) = selection_aabb(state) {
                                let (min2, max2) = project_aabb_to_2d(&sel, state.ortho_axis);
                                let center = [
                                    (min2.x as f32 + max2.x as f32) * 0.5,
                                    (min2.y as f32 + max2.y as f32) * 0.5,
                                ];
                                let off2 =
                                    project_to_2d(state.view2d_move_offset, state.ortho_axis);
                                let a = to_screen_f(center);
                                let b = to_screen_f([center[0] + off2[0], center[1] + off2[1]]);
                                (a, b)
                            } else {
                                (to_screen(start), to_screen(end))
                            };

                            let off = state.view2d_move_offset;
                            let (dx, dy) = match state.ortho_axis {
                                Ortho::XY => (off.x, off.y),
                                Ortho::XZ => (off.x, -off.z),
                                Ortho::YZ => (off.y, -off.z),
                            };
                            let delta_info = format!("({}, {})", format_float(dx, 2), format_float(dy, 2));

                            //let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                            let tw = text_width(ui, &delta_info);
                            //let ry = state.view2d_rect[1];
                            let text_h = text_height(ui, "1") + 2.0;
                            let d_info_pos = [b[0] - tw / 2.0, b[1] - text_h];
                            //let d_info_pos = [mid[0] - tw / 2.0, mid[1] - 18.0];

                            let delta_info_col = util::imgui_color_to_u32(col);

                            draw.add_line(a, b, util::adjust_color_brightness(delta_info_col, 1.5))
                                .thickness(2.0)
                                .build();
                            draw.add_text(
                                d_info_pos,
                                util::adjust_color_brightness(delta_info_col, 2.0),
                                delta_info,
                            );
                        }
                        DragMode::NewBrush => {
                            let min = start.min(end);
                            let max = start.max(end);

                            let a = to_screen(min);
                            let b = to_screen(max);

                            draw.add_rect(a, b, util::imgui_color_to_u32(col))
                                .thickness(2.0)
                                .build();
                        }
                        DragMode::StretchSelection => {
                            if let Some(stretch) = state.view2d_stretch.as_ref() {
                                let preview = editing::preview_stretched_aabb(
                                    &stretch.selection_aabb,
                                    stretch.faces,
                                    state.view2d_stretch_delta,
                                );
                                let (min2, max2) =
                                    project_aabb_to_2d(&preview, state.ortho_axis);
                                let a = to_screen(min2);
                                let b = to_screen(max2);

                                if let (Some(p0), Some(p1)) = (
                                    stretch_handle_point_2d(
                                        &stretch.selection_aabb,
                                        state.ortho_axis,
                                        stretch.faces,
                                    ),
                                    stretch_handle_point_2d(&preview, state.ortho_axis, stretch.faces),
                                ) {
                                    let a = to_screen_f(p0);
                                    let b = to_screen_f(p1);

                                    let du = format_float(p1[0] - p0[0], 2);
                                    let dv = format_float(p1[1] - p0[1], 2);
                                    let delta_info = format!("({}, {})", du, dv);

                                    let tw = text_width(ui, &delta_info);
                                    //let ry = state.view2d_rect[1];
                                    //let text_h = text_height(ui, "1") + 2.0;
                                    let d_info_pos = [b[0] - tw - 12.0, b[1] + 4.0];

                                    let delta_info_col = util::imgui_color_to_u32(col);

                                    draw.add_line(a, b, util::adjust_color_brightness(delta_info_col, 1.5))
                                    .thickness(2.0)
                                    .build();
                                    draw.add_text(
                                        d_info_pos,
                                        util::adjust_color_brightness(delta_info_col, 2.0),
                                        delta_info,
                                    );
                                }

                                draw.add_rect(a, b, util::imgui_color_to_u32(col))
                                    .thickness(2.0)
                                    .build();
                            } else {
                                let min = start.min(end);
                                let max = start.max(end);
                                let a = to_screen(min);
                                let b = to_screen(max);
                                draw.add_rect(a, b, util::imgui_color_to_u32(col))
                                    .thickness(2.0)
                                    .build();
                            }
                        }
                        DragMode::RotateSelection => {
                            if let Some(rot) = state.view2d_rotate.as_ref() {
                                let a = to_screen_f(rot.pivot_uv);
                                let b = to_screen(end);

                                let deg = state.view2d_rotate_angle.to_degrees();
                                let delta_info = format!("{deg:.1}°");
                                let tw = text_width(ui, &delta_info);
                                let text_h = text_height(ui, "1") + 2.0;
                                let d_info_pos = [b[0] - tw / 2.0, b[1] - text_h];

                                let delta_info_col = util::imgui_color_to_u32(col);
                                draw.add_line(
                                    a,
                                    b,
                                    util::adjust_color_brightness(delta_info_col, 1.5),
                                )
                                .thickness(2.0)
                                .build();
                                draw.add_text(
                                    d_info_pos,
                                    util::adjust_color_brightness(delta_info_col, 2.0),
                                    delta_info,
                                );
                            }
                        }
                    }
                }

                if let Some(mut selection_aabb) = selection_aabb(state) {
                    if state.view2d_drag_mode == DragMode::StretchSelection {
                        if let Some(stretch) = state.view2d_stretch.as_ref() {
                            selection_aabb = editing::preview_stretched_aabb(
                                &stretch.selection_aabb,
                                stretch.faces,
                                state.view2d_stretch_delta,
                            );
                        }
                    } else if state.view2d_drag_mode == DragMode::RotateSelection {
                        if let Some(rot) = state.view2d_rotate.as_ref() {
                            let axis = match state.ortho_axis {
                                Ortho::XY => Vec3::Z,
                                Ortho::XZ => Vec3::Y,
                                Ortho::YZ => Vec3::X,
                            };
                            if let Some((_xform, preview)) = editing::rotate_selection_transform(
                                &rot.selection_aabb,
                                axis,
                                state.view2d_rotate_angle,
                            ) {
                                selection_aabb = preview;
                            }
                        }
                    }

                    let off = if state.view2d_drag_mode == DragMode::MoveSelection {
                        project_to_2d(state.view2d_move_offset, state.ortho_axis)
                    } else {
                        [0.0, 0.0]
                    };

                    let (min_x, max_x, min_y, max_y) = match state.ortho_axis {
                        Ortho::XY => (
                            selection_aabb.min.x + off[0],
                            selection_aabb.max.x + off[0],
                            selection_aabb.min.y + off[1],
                            selection_aabb.max.y + off[1],
                        ),
                        Ortho::XZ => (
                            selection_aabb.min.x + off[0],
                            selection_aabb.max.x + off[0],
                            -selection_aabb.max.z + off[1],
                            -selection_aabb.min.z + off[1],
                        ),
                        Ortho::YZ => (
                            selection_aabb.min.y + off[0],
                            selection_aabb.max.y + off[0],
                            -selection_aabb.max.z + off[1],
                            -selection_aabb.min.z + off[1],
                        ),
                    };

                    let width = (max_x - min_x).abs();
                    let height = (max_y - min_y).abs();

                    let col_u32 =
                        util::imgui_color_to_u32(ui.style_color(StyleColor::ButtonActive));
                    let to_screen_f = |v: [f32; 2]| -> [f32; 2] {
                        [
                            p[0] + w * 0.5 + state.view2d_pan[0] + v[0] * state.view2d_zoom,
                            p[1] + h * 0.5 + state.view2d_pan[1] + v[1] * state.view2d_zoom,
                        ]
                    };

                    let bottom = to_screen_f([(min_x + max_x) * 0.5, max_y]);
                    let right = to_screen_f([max_x, (min_y + max_y) * 0.5]);

                    let w_text = format_float(width, 2);
                    let h_text = format_float(height, 2);
                    let w_tw = text_width(ui, &w_text);
                    //let h_tw = text_width(ui, &h_text);

                    draw.add_text([bottom[0] - w_tw * 0.5, bottom[1] + 8.0], col_u32, w_text);
                    draw.add_text([right[0] + 12.0 - 4.0, right[1] - 7.0], col_u32, h_text);

                    // the handle bars (c) raph
                    let v0 = to_screen_f([max_x, min_y]);
                    let v1 = to_screen_f([max_x, max_y]);
                    draw.add_line([v0[0] + 3.0, v0[1]], [v1[0] + 3.0, v1[1]], col_u32)
                        .build(); //.thickness(1.0).build();

                    let h0 = to_screen_f([min_x, max_y]);
                    let h1 = to_screen_f([max_x, max_y]);
                    draw.add_line([h0[0], h0[1] + 3.5], [h1[0], h1[1] + 3.5], col_u32)
                        .build(); //.thickness(1.0).build();
                }
            });

            // brush outlines
        });
}

fn selection_aabb(state: &EditorState) -> Option<Aabb> {
    if state.selected_brushes.is_empty() {
        return None;
    }
    let map = state.map.as_ref()?;
    selection_aabb_from_map(map, &state.selected_brushes)
}

fn selection_aabb_from_map(map: &kradiant::map::Map, selected: &[(usize, usize)]) -> Option<Aabb> {
    if selected.is_empty() {
        return None;
    }

    let mut out = Aabb {
        min: Vec3::new(f32::MAX, f32::MAX, f32::MAX),
        max: Vec3::new(f32::MIN, f32::MIN, f32::MIN),
    };

    let mut any = false;
    for &(entity_idx, brush_idx) in selected {
        let Some(entity) = map.entities.get(entity_idx) else {
            continue;
        };
        let Some(brush) = entity.brushes.get(brush_idx) else {
            continue;
        };
        out.min = out.min.min(brush.aabb.min);
        out.max = out.max.max(brush.aabb.max);
        any = true;
    }
    any.then_some(out)
}

fn stretch_faces_from_start(axis: Ortho, aabb: &Aabb, start: Vec2) -> Vec<editing::StretchFace> {
    let (min_u, max_u, min_v, max_v) = match axis {
        Ortho::XY => (aabb.min.x, aabb.max.x, aabb.min.y, aabb.max.y),
        Ortho::XZ => (aabb.min.x, aabb.max.x, -aabb.max.z, -aabb.min.z),
        Ortho::YZ => (aabb.min.y, aabb.max.y, -aabb.max.z, -aabb.min.z),
    };

    let mut out = Vec::with_capacity(2);

    if start.x < min_u {
        out.push(match axis {
            Ortho::XY | Ortho::XZ => editing::StretchFace::XMin,
            Ortho::YZ => editing::StretchFace::YMin,
        });
    } else if start.x > max_u {
        out.push(match axis {
            Ortho::XY | Ortho::XZ => editing::StretchFace::XMax,
            Ortho::YZ => editing::StretchFace::YMax,
        });
    }

    if start.y < min_v {
        out.push(match axis {
            Ortho::XY => editing::StretchFace::YMin,
            Ortho::XZ | Ortho::YZ => editing::StretchFace::ZMax,
        });
    } else if start.y > max_v {
        out.push(match axis {
            Ortho::XY => editing::StretchFace::YMax,
            Ortho::XZ | Ortho::YZ => editing::StretchFace::ZMin,
        });
    }

    out
}

fn update_last_work_from_aabb(state: &mut EditorState, aabb: &Aabb) {
    state.work_pos = (aabb.min + aabb.max) / 2.0;
    let d = aabb.max - aabb.min;
    let fallback = (state.config.grid_minor_step).max(1) as f32;
    state.work_depth = Vec3::new(
        normalize_depth(d.x, fallback),
        normalize_depth(d.y, fallback),
        normalize_depth(d.z, fallback),
    );
}

fn create_brush_from_drag(state: &mut EditorState, start: Vec2, end: Vec2) -> Option<BrushId> {
    let min2 = start.min(end);
    let max2 = start.max(end);
    if min2.x == max2.x || min2.y == max2.y {
        return None;
    }

    let fallback = (state.config.grid_minor_step as f32).max(1.0);
    let last = state.last_aabb.as_ref();
    let (min3, max3) = match state.ortho_axis {
        Ortho::XY => {
            let (z0, z1) = last.map(|a| (a.min.z, a.max.z)).unwrap_or((0.0, fallback));
            (
                Vec3::new(min2.x, min2.y, z0),
                Vec3::new(max2.x, max2.y, z1),
            )
        }
        Ortho::XZ => {
            let (y0, y1) = last.map(|a| (a.min.y, a.max.y)).unwrap_or((0.0, fallback));
            (
                Vec3::new(min2.x, y0, -max2.y),
                Vec3::new(max2.x, y1, -min2.y),
            )
        }
        Ortho::YZ => {
            let (x0, x1) = last.map(|a| (a.min.x, a.max.x)).unwrap_or((0.0, fallback));
            (
                Vec3::new(x0, min2.x, -max2.y),
                Vec3::new(x1, max2.x, -min2.y),
            )
        }
    };

    if state.map.is_none() {
        state.map = Some(kradiant::map::Map::default());
    }
    let Some(map) = state.map.as_mut() else {
        return None;
    };

    let aabb = editing::Aabb::from_points(min3, max3);
    match editing::add_convex_brush_from_aabb(map, 0, aabb, "common/caulk") {
        Ok(id) => {
            state.log_info(format!("Created brush {:?}", id));
            Some(id)
        }
        Err(e) => {
            state.log_error(format!("Failed to create brush: {e}"));
            None
        }
    }
}
/*
fn draw_ortho_grid(
    draw: &dear_imgui_rs::DrawListMut<'_>,
    origin: [f32; 2],
    w: f32,
    h: f32,
    zoom: f32,
    pan: [f32; 2],
    minor_step: u8,
) {
    let major_world = 64.0;
    let minor_world = minor_step.max(1);

    let major_step = major_world * zoom;
    let minor_step = minor_world as f32 * zoom;

    if major_step < 3.0 {
        return;
    }

    let cx = origin[0] + w * 0.5 + pan[0];
    let cy = origin[1] + h * 0.5 + pan[1];

    // --- MINOR GRID ---
    if minor_step >= 3.0 {
        let i0 = ((origin[0] - cx) / minor_step).floor() as i32 - 1;
        let i1 = ((origin[0] + w - cx) / minor_step).ceil() as i32 + 1;

        for i in i0..=i1 {
            let x = cx + i as f32 * minor_step;

            // skip lines where major grid will draw
            if (i as f32 % (major_world / minor_world as f32)) == 0.0 {
                continue;
            }

            draw.add_line([x, origin[1]], [x, origin[1] + h], 0xFF22_2222u32)
                .thickness(1.0)
                .build();
        }

        let j0 = ((origin[1] - cy) / minor_step).floor() as i32 - 1;
        let j1 = ((origin[1] + h - cy) / minor_step).ceil() as i32 + 1;

        for j in j0..=j1 {
            let y = cy + j as f32 * minor_step;

            if (j as f32 % (major_world / minor_world as f32)) == 0.0 {
                continue;
            }

            draw.add_line([origin[0], y], [origin[0] + w, y], 0xFF22_2222u32)
                .thickness(1.0)
                .build();
        }
    }

    // --- MAJOR GRID ---
    let i0 = ((origin[0] - cx) / major_step).floor() as i32 - 1;
    let i1 = ((origin[0] + w - cx) / major_step).ceil() as i32 + 1;

    for i in i0..=i1 {
        let x = cx + i as f32 * major_step;

        draw.add_line([x, origin[1]], [x, origin[1] + h], 0xFF2C_2C2Cu32)
            .thickness(1.0)
            .build();
    }

    let j0 = ((origin[1] - cy) / major_step).floor() as i32 - 1;
    let j1 = ((origin[1] + h - cy) / major_step).ceil() as i32 + 1;

    for j in j0..=j1 {
        let y = cy + j as f32 * major_step;

        draw.add_line([origin[0], y], [origin[0] + w, y], 0xFF2C_2C2Cu32)
            .thickness(1.0)
            .build();
    }
}*/

// Console

fn draw_console(ui: &Ui, state: &mut EditorState) {
    ui.window("Console")
        .size([1280.0, 180.0], Condition::FirstUseEver)
        .build(|| {
            if ui.small_button("Clear") {
                state.log.clear();
            }
            ui.same_line();
            ui.text("Filter:");
            ui.same_line();
            ui.set_next_item_width(180.0);
            let mut fbuf = state.con_filter.clone();
            if ui
                .input_text("##con_filter", &mut fbuf)
                .hint("search…")
                .build()
            {
                state.con_filter = fbuf;
            }

            ui.separator();

            ui.child_window("##console_log")
                .size(ui.content_region_avail())
                .build(ui, || {
                    let filter_lc = state.con_filter.to_ascii_lowercase();
                    for entry in &state.log {
                        if !filter_lc.is_empty()
                            && !entry.text.to_ascii_lowercase().contains(&filter_lc)
                        {
                            continue;
                        }
                        let col = match entry.level {
                            LogLevel::Info => state.palette.console_info,
                            LogLevel::Warn => state.palette.console_warn,
                            LogLevel::Error => state.palette.console_error,
                        };
                        let _tok = ui.push_style_color(StyleColor::Text, col);
                        ui.text(format!("{} {}", entry.level.prefix(), entry.text));
                        // _tok drops → pops colour
                    }
                    if state.console_scroll {
                        ui.set_scroll_here_y(1.0);
                        state.console_scroll = false;
                    }
                });
        });
}

// Texture Browser

fn draw_texture_browser(ui: &Ui, state: &mut EditorState) {
    ui.window("Textures")
        .size([1280.0, 200.0], Condition::FirstUseEver)
        .build(|| {
            ui.text("Filter:");
            ui.same_line();
            ui.set_next_item_width(200.0);
            let mut fbuf = state.tex_filter.clone();
            if ui
                .input_text("##tex_filter", &mut fbuf)
                .hint("e.g. caulk")
                .build()
            {
                state.tex_filter = fbuf;
            }
            ui.same_line();
            ui.text("  Size:");
            ui.same_line();
            ui.set_next_item_width(100.0);
            ui.slider_config("##tile_size", 32.0f32, 256.0f32)
                .display_format("%.0f px")
                .build(&mut state.tex_tile_size);

            if let Some(sel) = &state.tex_selected {
                ui.same_line();
                ui.text_disabled(format!("  selected: {sel}"));
            }

            ui.separator();

            ui.child_window("##tex_scroll")
                .size([0.0, 0.0])
                .build(ui, || draw_texture_tiles(ui, state));
        });
}

fn draw_texture_tiles(ui: &Ui, state: &mut EditorState) {
    // Placeholder list — replace with AssetDb::collect_used_materials() later
    let materials: &[&str] = &[
        "common/caulk",
        "common/clip",
        "common/trigger",
        "common/water",
    ];

    let tile = state.tex_tile_size;
    let label_h = ui.frame_height_with_spacing();
    let cell_w = tile + 4.0;
    let avail_w = ui.content_region_avail()[0].max(cell_w);
    let cols = ((avail_w + 4.0) / cell_w).floor().max(1.0) as usize;

    let filter_lc = state.tex_filter.to_ascii_lowercase();
    let visible: Vec<&str> = materials
        .iter()
        .filter(|m| filter_lc.is_empty() || m.to_ascii_lowercase().contains(&filter_lc))
        .copied()
        .collect();

    if visible.is_empty() {
        ui.text_disabled("no textures match filter");
        return;
    }

    for (i, material) in visible.iter().enumerate() {
        let short = material.rsplit('/').next().unwrap_or(material);
        let selected = state.tex_selected.as_deref() == Some(material);

        if selected {
            let p = ui.cursor_screen_pos();
            ui.get_window_draw_list()
                .add_rect(
                    p,
                    [p[0] + tile + 2.0, p[1] + tile + label_h + 2.0],
                    util::imgui_color_to_u32(ui.style_color(StyleColor::TabSelectedOverline)),
                )
                .filled(true)
                .rounding(3.0)
                .build();
        }

        // Deterministic colour placeholder — swap for Image::new(gpu_tex_id, [tile,tile]) later.
        {
            let hash = short
                .bytes()
                .fold(5381u32, |a, b| a.wrapping_mul(33).wrapping_add(b as u32));
            let r = (((hash) & 0x7F) as f32 + 64.0) / 255.0;
            let g = (((hash >> 8) & 0x7F) as f32 + 64.0) / 255.0;
            let b = (((hash >> 16) & 0x7F) as f32 + 64.0) / 255.0;
            // Pack as ABGR u32 that DrawList expects (0xAA_BB_GG_RR).
            let col = pack_abgr(r, g, b, 1.0);
            let p = ui.cursor_screen_pos();
            ui.get_window_draw_list()
                .add_rect(p, [p[0] + tile, p[1] + tile], col)
                .filled(true)
                .build();
        }

        if ui.invisible_button(format!("##t_{i}"), [tile, tile]) {
            state.tex_selected = if selected {
                None
            } else {
                Some(material.to_string())
            };
        }
        if ui.is_item_hovered() {
            ui.tooltip_text(*material);
        }

        //let label = truncate_to_width(ui, short, tile);
        //ui.text(&label);

        if (i + 1) % cols != 0 {
            ui.same_line_with_spacing(0.0, 4.0);
        }
    }
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
