use dear_imgui_rs::Ui;
use kradiant::editing::{self, Aabb};
use kradiant::loader::map_loader;
use num_traits::{Num, NumCast, ToPrimitive};
use std::fs::create_dir_all;
use std::io;
use std::path::PathBuf;

use crate::ui::{EditorState, Ortho};
use glam::{IVec2, IVec3, Vec3};

pub(crate) fn get_config_dir() -> io::Result<PathBuf> {
    let base = std::env::home_dir()
        .unwrap()
        .join(".config/kradiant_editor");
    if !base.exists() {
        create_dir_all(&base)?;
    }

    Ok(base)
}

/// Measure text width via the imgui sys layer (calc_text_size is not on &Ui in 0.10).
pub fn text_width(ui: &Ui, text: &str) -> f32 {
    let _ = ui; // keep signature symmetric with the rest
    let c = std::ffi::CString::new(text).unwrap_or_default();
    unsafe {
        // In this build igCalcTextSize(text, text_end, hide_text_after_double_hash, wrap_width)
        // and returns ImVec2 by value (no out-param).
        let sz = dear_imgui_rs::sys::igCalcTextSize(c.as_ptr(), std::ptr::null(), false, -1.0);
        sz.x
    }
}

/// Measure text height via the imgui sys layer (calc_text_size is not on &Ui in 0.10).
pub fn text_height(ui: &Ui, text: &str) -> f32 {
    let _ = ui; // keep signature symmetric with the rest
    let c = std::ffi::CString::new(text).unwrap_or_default();
    unsafe {
        // In this build igCalcTextSize(text, text_end, hide_text_after_double_hash, wrap_width)
        // and returns ImVec2 by value (no out-param).
        let sz = dear_imgui_rs::sys::igCalcTextSize(c.as_ptr(), std::ptr::null(), false, -1.0);
        sz.y
    }
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

/// Pack f32 RGBA into a u32 in the ABGR byte order that ImGui DrawList uses.
pub fn pack_abgr(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let ri = (r.clamp(0.0, 1.0) * 255.0).round() as u32;
    let gi = (g.clamp(0.0, 1.0) * 255.0).round() as u32;
    let bi = (b.clamp(0.0, 1.0) * 255.0).round() as u32;
    let ai = (a.clamp(0.0, 1.0) * 255.0).round() as u32;
    (ai << 24) | (bi << 16) | (gi << 8) | ri
}

pub fn imgui_color_to_u32(c: [f32; 4]) -> u32 {
    let r = (c[0] * 255.0) as u32;
    let g = (c[1] * 255.0) as u32;
    let b = (c[2] * 255.0) as u32;
    let a = (c[3] * 255.0) as u32;

    (a << 24) | (b << 16) | (g << 8) | r
}

/// Adjust brightness of an ImGui u32 color (format: 0xAARRGGBB).
/// `brightness` multiplies RGB channels (1.0 = unchanged, <1.0 darker, >1.0 brighter).
pub fn adjust_color_brightness(color: u32, brightness: f32) -> u32 {
    let a = ((color >> 24) & 0xFF) as u8;
    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;

    let scale = |v: u8| -> u8 {
        let v = (v as f32 * brightness).round();
        if v < 0.0 {
            0
        } else if v > 255.0 {
            255
        } else {
            v as u8
        }
    };

    let r = scale(r);
    let g = scale(g);
    let b = scale(b);

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

    [(mouse[0] - cx) / zoom, (mouse[1] - cy) / zoom]
}

pub fn project_to_2d(v: Vec3, axis: Ortho) -> [f32; 2] {
    match axis {
        Ortho::XY => [v.x, v.y],
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

pub fn new_map(state: &mut EditorState) {
    state.map_path = String::new();
    state.map = None;
    editor_log!(state, info, "New map");
}

pub fn open_map(state: &mut EditorState) {
    let cwd = std::env::current_dir().unwrap();
    let p = rfd::FileDialog::new()
        .set_title("Open a map")
        .add_filter("CoD Map", &["map", "bak"])
        .set_directory(cwd)
        .pick_file();

    if p.is_some() {
        let path = p.unwrap();
        let path_str = path.to_str().unwrap();
        match map_loader::load_map(path_str) {
            Ok(map) => {
                state.selected_brushes.clear();
                state.selected_entity = None;
                state.map_path = path_str.to_string();
                state.map = Some(map);
                editor_log!(state, info, "Loaded map: {}", path_str);
            }
            Err(e) => {
                editor_log!(
                    state,
                    error,
                    "Failed to load {}: {}",
                    path_str,
                    e.to_string()
                );
            }
        }
    }
}

pub fn save_map_as(state: &mut EditorState) {
    if state.map.is_none() {
        editor_log!(state, info, "Not allowed to save empty map!");
        return;
    }

    let cwd = std::env::current_dir().unwrap();
    match rfd::FileDialog::new()
        .set_title("Save map as")
        .add_filter("CoD Map", &["map", "bak"])
        .set_directory(cwd)
        .save_file()
    {
        Some(p) => {
            state.map_path = p.to_str().unwrap().to_string();
            save_map(state);
        }
        None => {
            editor_log!(state, info, "Save map cancelled by user");
            return;
        }
    }
}

pub fn save_map(state: &mut EditorState) {
    if state.map.is_none() {
        editor_log!(state, info, "Not allowed to save empty map!");
        return;
    }
    let mut chosen_path = PathBuf::new();
    let mut choose_new_file = if !state.map_path.is_empty() {
        let path = PathBuf::from(&state.map_path);
        if path.exists() && path.is_file() {
            chosen_path = path;
            false
        } else {
            true
        }
    } else {
        true
    };

    while choose_new_file {
        let cwd = std::env::current_dir().unwrap();
        match rfd::FileDialog::new()
            .set_title("Save map")
            .add_filter("CoD Map", &["map", "bak"])
            .set_directory(cwd)
            .save_file()
        {
            Some(p) => {
                chosen_path = p;
                choose_new_file = false;
            }
            None => {
                editor_log!(state, info, "Save map cancelled by user");
                return;
            }
        }
    }

    let path_str = chosen_path.to_str().unwrap();
    if let Some(map) = state.map.as_mut() {
        let changed = kradiant::editing::orient_map_convex_brushes_inward(map);
        if changed > 0 {
            editor_log!(state, info, "Oriented {} brushes", changed);
        }
    }

    match map_loader::save_map(state.map.as_ref().unwrap(), path_str) {
        Ok(_) => editor_log!(state, info, "Saved map to {}", path_str),
        Err(e) => {
            editor_log!(
                state,
                error,
                "Failed to save map to {}: {}",
                path_str,
                e.to_string()
            );
        }
    }
}

pub fn click_in_selection_aabb(state: &EditorState, pt: IVec2) -> bool {
    if state.selected_brushes.is_empty() {
        return false;
    }
    let Some(map) = &state.map else {
        return false;
    };

    let mut min = IVec3::new(i32::MAX, i32::MAX, i32::MAX);
    let mut max = IVec3::new(i32::MIN, i32::MIN, i32::MIN);
    let mut any = false;

    for &(entity_idx, brush_idx) in &state.selected_brushes {
        let Some(entity) = map.entities.get(entity_idx) else {
            continue;
        };
        let Some(brush) = entity.brushes.get(brush_idx) else {
            continue;
        };
        min = min.min(brush.aabb.min);
        max = max.max(brush.aabb.max);
        any = true;
    }
    if !any {
        return false;
    }

    let (min_x, max_x, min_y, max_y) = match state.ortho_axis {
        Ortho::XY => (min.x, max.x, min.y, max.y),
        Ortho::XZ => (min.x, max.x, -max.z, -min.z),
        Ortho::YZ => (min.y, max.y, -max.z, -min.z),
    };

    pt.x >= min_x && pt.x <= max_x && pt.y >= min_y && pt.y <= max_y
}

pub fn drag_delta_to_3d(d: IVec2, axis: Ortho) -> IVec3 {
    match axis {
        Ortho::XY => IVec3::new(d.x, d.y, 0),
        Ortho::XZ => IVec3::new(d.x, 0, -d.y),
        Ortho::YZ => IVec3::new(0, d.x, -d.y),
    }
}

pub fn div_ceil_i32(a: i32, b: i32) -> i32 {
    debug_assert!(b > 0);
    -((-a).div_euclid(b))
}

pub fn normalize_depth(v: i32, fallback: i32) -> i32 {
    let v = v.abs();
    if v == 0 { fallback.max(1) } else { v }
}

pub fn project_aabb_to_2d(aabb: &Aabb, axis: Ortho) -> (IVec2, IVec2) {
    match axis {
        Ortho::XY => (
            IVec2::new(aabb.min.x, aabb.min.y),
            IVec2::new(aabb.max.x, aabb.max.y),
        ),
        Ortho::XZ => (
            IVec2::new(aabb.min.x, -aabb.max.z),
            IVec2::new(aabb.max.x, -aabb.min.z),
        ),
        Ortho::YZ => (
            IVec2::new(aabb.min.y, -aabb.max.z),
            IVec2::new(aabb.max.y, -aabb.min.z),
        ),
    }
}

pub fn clamp_stretch_delta(
    aabb: &Aabb,
    faces: [Option<editing::StretchFace>; 2],
    delta: IVec3,
    grid_step: i32,
) -> IVec3 {
    let step = grid_step.abs().max(1);
    let mut out = delta;

    for face in faces.iter().flatten() {
        match face {
            editing::StretchFace::XMin => {
                if out.x > 0 {
                    let max_delta = (aabb.max.x - 1) - aabb.min.x;
                    if out.x > max_delta {
                        out.x = (max_delta.div_euclid(step) * step).max(0);
                    }
                }
            }
            editing::StretchFace::XMax => {
                if out.x < 0 {
                    let min_delta = (aabb.min.x + 1) - aabb.max.x;
                    if out.x < min_delta {
                        out.x = div_ceil_i32(min_delta, step) * step;
                    }
                }
            }
            editing::StretchFace::YMin => {
                if out.y > 0 {
                    let max_delta = (aabb.max.y - 1) - aabb.min.y;
                    if out.y > max_delta {
                        out.y = (max_delta.div_euclid(step) * step).max(0);
                    }
                }
            }
            editing::StretchFace::YMax => {
                if out.y < 0 {
                    let min_delta = (aabb.min.y + 1) - aabb.max.y;
                    if out.y < min_delta {
                        out.y = div_ceil_i32(min_delta, step) * step;
                    }
                }
            }
            editing::StretchFace::ZMin => {
                if out.z > 0 {
                    let max_delta = (aabb.max.z - 1) - aabb.min.z;
                    if out.z > max_delta {
                        out.z = (max_delta.div_euclid(step) * step).max(0);
                    }
                }
            }
            editing::StretchFace::ZMax => {
                if out.z < 0 {
                    let min_delta = (aabb.min.z + 1) - aabb.max.z;
                    if out.z < min_delta {
                        out.z = div_ceil_i32(min_delta, step) * step;
                    }
                }
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

            (Ortho::XY, editing::StretchFace::YMin) => v_side = Some(false),
            (Ortho::XY, editing::StretchFace::YMax) => v_side = Some(true),

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
}

pub fn to_num<T, U>(n: U) -> T
where
    T: NumCast + Default,
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
