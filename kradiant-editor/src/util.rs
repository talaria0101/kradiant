use dear_imgui_rs::Ui;
use kradiant::loader::map_loader;
use std::fs::create_dir_all;
use std::io;
use std::path::PathBuf;

use crate::ui::{EditorState, Ortho};
use glam::Vec3;

pub(crate) fn get_config_dir() -> io::Result<PathBuf> {
    let base = std::env::home_dir()
        .unwrap()
        .join(".config/kradiant_editor");
    if !base.exists() {
        create_dir_all(&base)?;
    }

    Ok(base)
}

// Helpers

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
                state.map_path = path_str.to_string();
                state.map = Some(map);
                editor_log_e!(state, info, "Loaded map: {}", path_str);
            }
            Err(e) => {
                editor_log_e!(
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

pub fn save_map(state: &mut EditorState) {
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
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Save map")
            .add_filter("CoD Map", &["map", "bak"])
            .set_directory(cwd)
            .save_file()
        {
            chosen_path = p;
            choose_new_file = false;
        }
    }

    let path_str = chosen_path.to_str().unwrap();
    match map_loader::save_map(state.map.as_ref().unwrap(), path_str) {
        Ok(_) => editor_log_e!(state, info, "Saved map to {}", path_str),
        Err(e) => {
            editor_log_e!(
                state,
                error,
                "Failed to save map to {}: {}",
                path_str,
                e.to_string()
            );
        }
    }
}

pub fn imgui_color_to_u32(c: [f32; 4]) -> u32 {
    let r = (c[0] * 255.0) as u32;
    let g = (c[1] * 255.0) as u32;
    let b = (c[2] * 255.0) as u32;
    let a = (c[3] * 255.0) as u32;

    (a << 24) | (b << 16) | (g << 8) | r
}
