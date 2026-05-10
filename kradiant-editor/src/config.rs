use std::{
    fs::OpenOptions,
    io::{Read, Write},
};

use crate::{ui::console::ConsoleLogger, util};
use kradiant::editor::config::EditorConfig;
pub use kradiant::editor::config::RenderMode;
use num_traits::NumCast;
use strum::VariantArray;

pub fn load() -> (EditorConfig, Option<String>) {
    let cfg_path = util::get_config_dir()
        .expect("Failed to get config dir")
        .join("prefs.toml");
    if !cfg_path.exists() {
        return (EditorConfig::default(), None);
    }
    let mut cfg_file = OpenOptions::new()
        .read(true)
        .open(&cfg_path)
        .expect("Failed to open config");
    let mut cfg_str = String::new();
    cfg_file
        .read_to_string(&mut cfg_str)
        .expect("Failed to read config");
    match toml::from_str::<EditorConfig>(&cfg_str) {
        Ok(cfg) => (cfg, None),
        Err(e) => (
            EditorConfig::default(),
            Some(format!(
                "Failed to load config from {}: {e}",
                cfg_path.display()
            )),
        ),
    }
}

pub fn save(cfg: &EditorConfig) -> String {
    let cfg_str: String = toml::to_string(cfg).expect("Serialize config");
    let cfg_path = util::get_config_dir()
        .expect("Failed to get config dir")
        .join("prefs.toml");
    let mut cfg_file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(&cfg_path)
        .expect("Failed to open config");
    match cfg_file.write(cfg_str.as_bytes()) {
        Ok(_) => format!("Saved configuration to {}", cfg_path.display()),
        Err(e) => format!(
            "Error saving configuration to {}: {}",
            cfg_path.display(),
            e
        ),
    }
}

pub fn update(
    cfg: &mut EditorConfig,
    key: &str,
    value: impl NumCast,
    console: &mut ConsoleLogger,
    view_config_rev: &mut u64,
) {
    match key {
        "grid_snap" => {
            let value_u8: u8 = util::to_num(value);
            cfg.view.grid_snap = value_u8 == 1;
            log_info!(console, "Set grid snap to {}", cfg.view.grid_snap);
        }
        "grid_minor_step" => {
            cfg.view.grid_minor_step = util::to_num(value);
            log_info!(console, "Set grid step to {}", cfg.view.grid_minor_step);
        }
        "view3d_fov" => {
            cfg.view.fov = util::to_num(value);
            log_info!(console, "Set 3d view fov to {}", cfg.view.fov);
        }
        "active_theme" => {
            cfg.misc.theme = util::to_num(value);
            log_info!(console, "Set active theme index to {}", cfg.misc.theme);
        }
        "wireframe" => {
            let value_u8: u8 = util::to_num(value);
            cfg.view.wireframe = value_u8 == 1;
            *view_config_rev = view_config_rev.wrapping_add(1);
            log_info!(console, "Set wireframe to {}", cfg.view.wireframe);
        }
        "rendermode" => {
            let value_usize: usize = util::to_num(value);
            let mode = RenderMode::VARIANTS
                .get(value_usize)
                .cloned()
                .unwrap_or(RenderMode::Flat);
            cfg.view.rendermode = mode;
            *view_config_rev = view_config_rev.wrapping_add(1);
            log_info!(
                console,
                "Set rendermode to {}",
                cfg.view.rendermode.as_ref()
            );
        }
        _ => (),
    }
}
