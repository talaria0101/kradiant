use dear_imgui_rs::Ui;
use glam::{Mat4, Vec2, Vec3};
use kradiant::dirs;
use kradiant::editing::{self, Aabb};
use kradiant::editor::config::EntityDef;
use kradiant::editor::viewport::Ortho;
use kradiant::loader::map_loader;
use kradiant::map::Map;
use num_traits::{NumCast, ToPrimitive};
use std::collections::BTreeMap;
use std::fs::{self, create_dir_all, read_to_string};
use std::io;
use std::path::PathBuf;

use crate::ui::EditorState;
use crate::ui::console::ConsoleLogger;
use kradiant::editor::selection::{EdgeSelection, FaceSelection, PatchVertexSelection};
use kradiant::editor::undo::UndoRedo;

pub fn get_config_dir() -> io::Result<PathBuf> {
    let base = std::env::home_dir()
        .unwrap()
        .join(".config/kradiant_editor");
    if !base.exists() {
        create_dir_all(&base)?;
    }

    Ok(base)
}

pub fn read_cfg(f: &str) -> io::Result<String> {
    let user_path = dirs::config_home()
    .join("kradiant_editor")
    .join(f);

    match read_to_string(&user_path) {
        Ok(s) if !s.is_empty() => return Ok(s),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e)
    }

    // system config dirs
    for dir in dirs::config_dirs() {
        let path = dir.join("kradiant_editor").join("prefs.toml");
        match read_to_string(&path) {
            Ok(s) if !s.is_empty() => return Ok(s),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }

    Err(io::Error::new(io::ErrorKind::NotFound, "Config not found anywhere"))
}

pub fn write_cfg(f: &str, c: &str) -> io::Result<()> {
    let cfg_path = dirs::config_home().join("kradiant_editor").join(f);
    fs::write(cfg_path, c.as_bytes())
}

/// Measure text dimensions via the imgui sys layer (calc_text_size is not on &Ui in 0.10).
/// Returns (width, height) tuple.
fn measure_text_impl(text: &str) -> (f32, f32) {
    let c = std::ffi::CString::new(text).unwrap_or_default();
    unsafe {
        // In this build igCalcTextSize(text, text_end, hide_text_after_double_hash, wrap_width)
        // returns ImVec2 by value (no out-param).
        let sz = dear_imgui_rs::sys::igCalcTextSize(c.as_ptr(), std::ptr::null(), false, -1.0);
        (sz.x, sz.y)
    }
}

/// Measure text width via the imgui sys layer.
pub fn text_width(_ui: &Ui, text: &str) -> f32 {
    measure_text_impl(text).0
}

/// Measure text height via the imgui sys layer.
pub fn text_height(_ui: &Ui, text: &str) -> f32 {
    measure_text_impl(text).1
}

/// Truncate `text` so it fits within `max_px`, appending `…` if needed.
pub fn truncate_to_width(_ui: &Ui, text: &str, max_px: f32) -> String {
    if text_width(_ui, text) <= max_px {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let probe = format!("{out}{ch}…");
        if text_width(_ui, &probe) > max_px {
            return format!("{out}…");
        }
        out.push(ch);
    }
    out
}

/// Convert normalized RGBA [0-1] to u32 ABGR format for ImGui.
pub fn pack_abgr(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let ri = (r.clamp(0.0, 1.0) * 255.0).round() as u32;
    let gi = (g.clamp(0.0, 1.0) * 255.0).round() as u32;
    let bi = (b.clamp(0.0, 1.0) * 255.0).round() as u32;
    let ai = (a.clamp(0.0, 1.0) * 255.0).round() as u32;
    (ai << 24) | (bi << 16) | (gi << 8) | ri
}

/// Convert RGBA color array [0-1] to u32 in ABGR byte order (same as pack_abgr).
pub fn imgui_color_to_u32(c: [f32; 4]) -> u32 {
    pack_abgr(c[0], c[1], c[2], c[3])
}

/// Adjust brightness of an ImGui u32 color (format: 0xAARRGGBB).
/// `brightness` multiplies RGB channels (1.0 = unchanged, <1.0 darker, >1.0 brighter).
pub fn adjust_color_brightness(color: u32, brightness: f32) -> u32 {
    let extract_channel = |shift: u8| ((color >> shift) & 0xFF) as u8;
    let a = extract_channel(24);
    let r = extract_channel(16);
    let g = extract_channel(8);
    let b = extract_channel(0);

    let scale = |v: u8| -> u8 {
        let scaled = (v as f32 * brightness).round();
        scaled.clamp(0.0, 255.0) as u8
    };

    let r = scale(r);
    let g = scale(g);
    let b = scale(b);

    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

pub fn adjust_color_opacity(color: u32, opacity: f32) -> u32 {
    let extract_channel = |shift: u8| ((color >> shift) & 0xFF) as u8;
    let a = extract_channel(24);
    let r = extract_channel(16);
    let g = extract_channel(8);
    let b = extract_channel(0);

    let scale = |v: u8| -> u8 {
        let scaled = (v as f32 * opacity).round();
        scaled.clamp(0.0, 255.0) as u8
    };

    let a = scale(a);

    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

pub fn screen_to_world(
    mouse: [f32; 2],
    origin: [f32; 2],
    size: [f32; 2],
    zoom: f32,
    pan: [f32; 2],
) -> [f32; 2] {
    let cx = origin[0] + size[0] * 0.5 + pan[0];
    let cy = origin[1] + size[1] * 0.5 + pan[1];

    // View2D "screen space" is Y-down (matches ImGui).
    [(mouse[0] - cx) / zoom, (mouse[1] - cy) / zoom]
}

pub fn project_to_2d(v: Vec3, axis: Ortho) -> [f32; 2] {
    match axis {
        // Flip Y so +Y is up on screen (ImGui Y+ is down).
        Ortho::XY => [v.x, -v.y],
        // Flip Z so +Z is up on screen (ImGui Y+ is down).
        Ortho::XZ => [v.x, -v.z],
        Ortho::YZ => [v.y, -v.z],
    }
}

pub fn world_to_screen(
    v: [f32; 2],
    p: [f32; 2],
    w: f32,
    h: f32,
    zoom: f32,
    pan: [f32; 2],
) -> [f32; 2] {
    [
        p[0] + w * 0.5 + pan[0] + v[0] * zoom,
        p[1] + h * 0.5 + pan[1] + v[1] * zoom,
    ]
}

pub fn world_to_screen_3d(vp: Mat4, rect: [f32; 4], world: Vec3) -> Option<[f32; 2]> {
    let clip = vp * glam::Vec4::new(world.x, world.y, world.z, 1.0);
    if clip.w < 0.1 {
        return None; // behind or too close to camera
    }
    let ndc = glam::Vec3::new(clip.x, clip.y, clip.z) / clip.w;
    // Reject points outside the clip volume (beyond any frustum face).
    if ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 || ndc.z.abs() > 1.0 {
        return None;
    }
    let [rx, ry, rw, rh] = rect;
    Some([
        rx + (ndc.x * 0.5 + 0.5) * rw,
        ry + (1.0 - (ndc.y * 0.5 + 0.5)) * rh,
    ])
}

pub fn snap(v: f32, step: f32) -> f32 {
    (v / step).round() * step
}
/*
/// Convert screen coordinates → world coordinates in current ortho view
pub fn screen_to_world_ortho(
    screen_pos: [f32; 2],
    view_origin: [f32; 2],
    view_size: [f32; 2],
    zoom: f32,
    pan: [f32; 2]
) -> [f32; 2]
{
    let rel_x = (screen_pos[0] - view_origin[0]) / view_size[0];
    let rel_y = (screen_pos[1] - view_origin[1]) / view_size[1];

    let world_half_w = view_size[0] / (2.0 * zoom);
    let world_half_h = view_size[1] / (2.0 * zoom);

    let world_x = pan[0] - world_half_w + rel_x * (2.0 * world_half_w);
    let world_y = pan[1] - world_half_h + rel_y * (2.0 * world_half_h);

    [world_x, world_y]
}
*/

fn confirm_discard(state: &EditorState) -> bool {
    if !state.core.dirty {
        return true;
    }
    rfd::MessageDialog::new()
        .set_title("Unsaved Changes")
        .set_description("The current map has unsaved changes. Discard them?")
        .set_level(rfd::MessageLevel::Warning)
        .set_buttons(rfd::MessageButtons::OkCancel)
        .show()
        == rfd::MessageDialogResult::Ok
}

pub fn new_map(state: &mut EditorState) {
    if !confirm_discard(state) {
        return;
    }
    state.core.map_path = "unsaved.map".to_string();
    state.core.map = Some(Map::default());
    state.core.bump_revision();
    state.core.dirty = false;
    state.core.selected_brushes.clear();
    state.core.selected_faces.clear();
    state.core.selected_edges.clear();
    state.core.selected_patch_vertices.clear();
    state.core.selected_entities.clear();
    state.core.undo.clear();
    state.core.map_load_count = state.core.map_load_count.wrapping_add(1);
    log_info!(state.console, "New map");
}

pub fn open_map(state: &mut EditorState) {
    if !confirm_discard(state) {
        return;
    }
    let cwd = {
        if !state.core.config.misc.recent_maps.is_empty() {
            let last = state.core.config.misc.recent_maps.last().unwrap();
            PathBuf::from(last)
                .parent()
                .unwrap_or(&PathBuf::from("/"))
                .to_path_buf()
        } else {
            std::env::current_dir().unwrap()
        }
    };
    let p = rfd::FileDialog::new()
        .set_title("Open a map")
        .add_filter("CoD Map", &["map", "bak"])
        .set_directory(cwd)
        .pick_file();

    if let Some(path) = p {
        let path_str = path.to_str().unwrap();
        perform_open_map(
            &mut state.core.selected_brushes,
            &mut state.core.selected_faces,
            &mut state.core.selected_edges,
            &mut state.core.selected_patch_vertices,
            &mut state.core.selected_entities,
            &mut state.core.map_path,
            &mut state.core.map,
            &mut state.core.map_revision,
            &mut state.core.dirty,
            &mut state.core.undo,
            &mut state.core.map_load_count,
            &mut state.core.config.misc.recent_maps,
            &mut state.console,
            path_str,
        );
    }
}

pub fn open_recent_map(state: &mut EditorState, path: &str) {
    if !confirm_discard(state) {
        return;
    }
    perform_open_map(
        &mut state.core.selected_brushes,
        &mut state.core.selected_faces,
        &mut state.core.selected_edges,
        &mut state.core.selected_patch_vertices,
        &mut state.core.selected_entities,
        &mut state.core.map_path,
        &mut state.core.map,
        &mut state.core.map_revision,
        &mut state.core.dirty,
        &mut state.core.undo,
        &mut state.core.map_load_count,
        &mut state.core.config.misc.recent_maps,
        &mut state.console,
        path,
    );
}

fn perform_open_map(
    selected_brushes: &mut Vec<(usize, usize)>,
    selected_faces: &mut Vec<FaceSelection>,
    selected_edges: &mut Vec<EdgeSelection>,
    selected_patch_vertices: &mut Vec<PatchVertexSelection>,
    selected_entities: &mut Vec<usize>,
    map_path: &mut String,
    map: &mut Option<Map>,
    map_revision: &mut u64,
    dirty: &mut bool,
    undo: &mut UndoRedo,
    map_load_count: &mut u64,
    recent_maps: &mut Vec<String>,
    console: &mut ConsoleLogger,
    path: &str,
) {
    match map_loader::load_map(path) {
        Ok(loaded_map) => {
            selected_brushes.clear();
            selected_faces.clear();
            selected_edges.clear();
            selected_patch_vertices.clear();
            selected_entities.clear();
            *map_path = path.to_string();
            *map = Some(loaded_map);
            *map_revision = map_revision.wrapping_add(1);
            *dirty = false;
            undo.clear();
            *map_load_count = map_load_count.wrapping_add(1);
            let path_str = path.to_string();
            recent_maps.retain(|p| p != &path_str);
            recent_maps.push(path_str);
            truncate_recent_maps(recent_maps);
            log_info!(console, "Loaded map: {}", path);
        }
        Err(e) => {
            log_error!(console, "Failed to load {}: {}", path, e.to_string());
        }
    }
}

fn truncate_recent_maps(recent_maps: &mut Vec<String>) {
    const MAX_RECENT: usize = 20;
    if recent_maps.len() > MAX_RECENT {
        let drain = recent_maps.len() - MAX_RECENT;
        recent_maps.drain(..drain);
    }
}

fn get_save_path(force_dialog: bool, current_path: &str) -> Option<PathBuf> {
    if !force_dialog && !current_path.is_empty() {
        let path = PathBuf::from(current_path);
        if path.exists() && path.is_file() {
            return Some(path);
        }
    }

    let start_dir = if !current_path.is_empty() {
        PathBuf::from(current_path)
            .parent()
            .unwrap_or(&PathBuf::from("/"))
            .to_path_buf()
    } else {
        std::env::current_dir().unwrap()
    };
    rfd::FileDialog::new()
        .set_title("Save map")
        .add_filter("CoD Map", &["map", "bak"])
        .set_directory(start_dir)
        .save_file()
}

pub fn save_map_as(state: &mut EditorState) {
    if state.core.map.is_none() {
        log_error!(state.console, "Not allowed to save empty map!");
        return;
    }

    if let Some(p) = get_save_path(true, "") {
        state.core.map_path = p.to_str().unwrap().to_string();
        perform_save_map(state, &p);
        state.core.dirty = false;
    } else {
        log_info!(state.console, "Save map cancelled by user");
    }
}

pub fn save_map(state: &mut EditorState) {
    if state.core.map.is_none() {
        log_error!(state.console, "Not allowed to save empty map!");
        return;
    }

    if let Some(path) = get_save_path(false, &state.core.map_path) {
        state.core.map_path = path.to_str().unwrap().to_string();
        perform_save_map(state, &path);
        state.core.dirty = false;
    } else {
        log_info!(state.console, "Save map cancelled by user");
    }
}

/// Perform the actual map save operation.
fn perform_save_map(state: &mut EditorState, path: &PathBuf) {
    let path_str = path.to_str().unwrap();
    if let Some(map) = state.core.map.as_mut() {
        let changed = kradiant::editing::orient_map_convex_brushes_inward(map);
        if changed > 0 {
            log_info!(state.console, "Oriented {} brushes", changed);
        }
    }

    match map_loader::save_map(state.core.map.as_ref().unwrap(), path_str) {
        Ok(_) => {
            let p = path_str.to_string();
            state.core.config.misc.recent_maps.retain(|x| x != &p);
            state.core.config.misc.recent_maps.push(p);
            truncate_recent_maps(&mut state.core.config.misc.recent_maps);
            log_info!(state.console, "Saved map to {}", path_str);
        }
        Err(e) => log_error!(state.console, "Failed to save map to {}: {}", path_str, e),
    }
}

use crate::ui::AxisLock;
pub fn drag_delta_to_3d(d: Vec2, axis: Ortho, lock: &AxisLock) -> Vec3 {
    match axis {
        Ortho::XY => {
            let x = if lock.x { 0.0 } else { d.x };
            // Screen-space drag is Y-down; world-space is Y-up.
            let y = if lock.y { 0.0 } else { -d.y };
            Vec3 { x, y, z: 0.0 }
        }
        Ortho::XZ => {
            let x = if lock.x { 0.0 } else { d.x };
            let z = if lock.z { 0.0 } else { -d.y };
            Vec3 { x, y: 0.0, z }
        }
        Ortho::YZ => {
            let y = if lock.y { 0.0 } else { d.x };
            let z = if lock.z { 0.0 } else { -d.y };
            Vec3 { x: 0.0, y, z }
        }
    }
}

pub fn normalize_depth(v: f32, fallback: f32) -> f32 {
    let v = v.abs();
    if v == 0.0 { fallback.max(1.0) } else { v }
}

pub fn project_aabb_to_2d(aabb: &Aabb, axis: Ortho) -> (Vec2, Vec2) {
    match axis {
        Ortho::XY => (
            Vec2::new(aabb.min.x, -aabb.max.y),
            Vec2::new(aabb.max.x, -aabb.min.y),
        ),
        Ortho::XZ => (
            Vec2::new(aabb.min.x, -aabb.max.z),
            Vec2::new(aabb.max.x, -aabb.min.z),
        ),
        Ortho::YZ => (
            Vec2::new(aabb.min.y, -aabb.max.z),
            Vec2::new(aabb.max.y, -aabb.min.z),
        ),
    }
}

pub fn clamp_stretch_delta(
    aabb: &Aabb,
    faces: [Option<editing::StretchFace>; 2],
    delta: Vec3,
    grid_step: i32,
    grid_snapping: bool,
) -> Vec3 {
    let step = grid_step.abs().max(1) as f32;
    let mut out = delta;

    for face in faces.iter().flatten() {
        match face {
            editing::StretchFace::XMin => {
                if grid_snapping {
                    let new_x = aabb.min.x + out.x;
                    let snapped_x = snap(new_x, step);
                    out.x = snapped_x - aabb.min.x;
                }
                // Always clamp to prevent inverting the brush
                let max_delta = (aabb.max.x - 1.0) - aabb.min.x;
                out.x = out.x.clamp(-f32::INFINITY, max_delta);
            }
            editing::StretchFace::XMax => {
                if grid_snapping {
                    let new_x = aabb.max.x + out.x;
                    let snapped_x = snap(new_x, step);
                    out.x = snapped_x - aabb.max.x;
                }
                // Always clamp to prevent inverting the brush
                let min_delta = (aabb.min.x + 1.0) - aabb.max.x;
                out.x = out.x.clamp(min_delta, f32::INFINITY);
            }
            editing::StretchFace::YMin => {
                if grid_snapping {
                    let new_y = aabb.min.y + out.y;
                    let snapped_y = snap(new_y, step);
                    out.y = snapped_y - aabb.min.y;
                }
                // Always clamp to prevent inverting the brush
                let max_delta = (aabb.max.y - 1.0) - aabb.min.y;
                out.y = out.y.clamp(-f32::INFINITY, max_delta);
            }
            editing::StretchFace::YMax => {
                if grid_snapping {
                    let new_y = aabb.max.y + out.y;
                    let snapped_y = snap(new_y, step);
                    out.y = snapped_y - aabb.max.y;
                }
                // Always clamp to prevent inverting the brush
                let min_delta = (aabb.min.y + 1.0) - aabb.max.y;
                out.y = out.y.clamp(min_delta, f32::INFINITY);
            }
            editing::StretchFace::ZMin => {
                if grid_snapping {
                    let new_z = aabb.min.z + out.z;
                    let snapped_z = snap(new_z, step);
                    out.z = snapped_z - aabb.min.z;
                }
                // Always clamp to prevent inverting the brush
                let max_delta = (aabb.max.z - 1.0) - aabb.min.z;
                out.z = out.z.clamp(-f32::INFINITY, max_delta);
            }
            editing::StretchFace::ZMax => {
                if grid_snapping {
                    let new_z = aabb.max.z + out.z;
                    let snapped_z = snap(new_z, step);
                    out.z = snapped_z - aabb.max.z;
                }
                // Always clamp to prevent inverting the brush
                let min_delta = (aabb.min.z + 1.0) - aabb.max.z;
                out.z = out.z.clamp(min_delta, f32::INFINITY);
            }
        }
    }

    out
}

pub fn stretch_handle_point_2d(
    aabb: &Aabb,
    axis: Ortho,
    faces: [Option<editing::StretchFace>; 2],
) -> Option<[f32; 2]> {
    let (min2, max2) = project_aabb_to_2d(aabb, axis);

    let mut u_side: Option<bool> = None; // false=min, true=max
    let mut v_side: Option<bool> = None;

    for face in faces.iter().flatten() {
        match (axis, face) {
            (Ortho::XY | Ortho::XZ, editing::StretchFace::XMin) => u_side = Some(false),
            (Ortho::XY | Ortho::XZ, editing::StretchFace::XMax) => u_side = Some(true),
            (Ortho::YZ, editing::StretchFace::YMin) => u_side = Some(false),
            (Ortho::YZ, editing::StretchFace::YMax) => u_side = Some(true),

            (Ortho::XY, editing::StretchFace::YMax) => v_side = Some(false),
            (Ortho::XY, editing::StretchFace::YMin) => v_side = Some(true),

            // In XZ/YZ, projected V is -Z: ZMax maps to min2.y, ZMin maps to max2.y.
            (Ortho::XZ | Ortho::YZ, editing::StretchFace::ZMax) => v_side = Some(false),
            (Ortho::XZ | Ortho::YZ, editing::StretchFace::ZMin) => v_side = Some(true),
            _ => {}
        }
    }

    let u = if let Some(max_side) = u_side {
        if max_side {
            max2.x as f32
        } else {
            min2.x as f32
        }
    } else {
        (min2.x as f32 + max2.x as f32) * 0.5
    };
    let v = if let Some(max_side) = v_side {
        if max_side {
            max2.y as f32
        } else {
            min2.y as f32
        }
    } else {
        (min2.y as f32 + max2.y as f32) * 0.5
    };

    Some([u, v])
}
/*
pub fn num_from_str<T: std::str::FromStr + std::default::Default + Num>(s: &str) -> T
where
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    match s.parse::<T>() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to parse string for numeric value: {}", e);
            T::default()
        }
    }
}*/
/*
pub trait ToMyNum { fn to(self) -> u8; }
impl ToMyNum for bool {
    fn to(self) -> u8
    {
        if self { 1 } else { 0 }
    }
}

impl<T: Num> ToMyNum for T {
    fn to(self) -> T
    {
        self
    }
}
*/

pub fn to_num<T, U>(n: U) -> T
where
    T: NumCast + Default + Copy,
    U: ToPrimitive,
{
    match NumCast::from(n) {
        Some(r) => r,
        None => {
            eprintln!("Failed to cast numeric value to desired type");
            T::default()
        }
    }
}

/// Center the next widget horizontally within the available width.
pub fn center_next(ui: &Ui, item_width: f32) {
    let avail = ui.content_region_avail()[0];
    let offset = (avail - item_width) * 0.5;
    if offset > 0.0 {
        ui.set_cursor_pos([ui.cursor_pos()[0] + offset, ui.cursor_pos()[1]]);
    }
}

pub fn other_corners(tl: [f32; 2], br: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    ([br[0], tl[1]], [tl[0], br[1]])
}
/*
pub fn get_ent_categories(ents: Vec<EntityDef>) -> BTreeMap<String, Vec<EntityDef>>
{
    let mut map: BTreeMap<String, Vec<EntityDef>> = BTreeMap::new();
    for ent in ents {
        let (cat, e) = {
            let split = ent.class.split_once("_").unwrap();
            (split.0, split.1.to_string())
        };
        if map.contains_key(cat) {
            if let Some(cat_vec) = map.get_mut(cat) {
                if !cat_vec.contains(&(e, ent)) {
                    cat_vec.push((e, ent));
                }
            }
        }
        else {
            map.insert(cat.to_string(), vec![(e, ent)]);
        }
    }

    map
}*/

pub fn get_ent_categories(ents: Vec<EntityDef>) -> BTreeMap<String, Vec<(String, EntityDef)>> {
    let mut map: BTreeMap<String, Vec<(String, EntityDef)>> = BTreeMap::new();
    for ent in ents {
        let Some((cat, e)) = ent.class.split_once('_') else {
            continue;
        };
        let cat = cat.to_string();
        let e = e.to_string();

        let cat_vec = map.entry(cat).or_insert_with(Vec::new);
        if !cat_vec.iter().any(|(name, _)| name == &e) {
            cat_vec.push((e, ent));
        }
    }

    map
}
