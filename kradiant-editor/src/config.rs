use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
};

use crate::{ui::console::ConsoleLogger, util};
pub use kradiant::editor::config::RenderMode;
use kradiant::editor::config::{EditorConfig, EditorEntities, EntityDrawingConfig};
use num_traits::NumCast;
use strum::VariantArray;

pub fn load() -> io::Result<EditorConfig> {
    let cfg_path = util::get_config_dir()
        .expect("Failed to get config dir")
        .join("prefs.toml");
    if !cfg_path.exists() {
        return Ok(EditorConfig::default());
    }
    let mut cfg_file = OpenOptions::new()
        .read(true)
        .open(&cfg_path)
        .expect("Failed to open config");
    let mut cfg_str = String::new();
    cfg_file
        .read_to_string(&mut cfg_str)
        .expect("Failed to read config");

    let cfg = toml::from_str::<EditorConfig>(&cfg_str);
    if let Ok(cfg) = cfg {
        Ok(cfg)
    } else {
        Err(io::Error::new(io::ErrorKind::Other, cfg.err().unwrap()))
    }
}

pub fn load_entity_drawing() -> io::Result<EntityDrawingConfig> {
    let cfg_path = util::get_config_dir()
        .expect("Failed to get config dir")
        .join("ent_drawing.toml");
    if !cfg_path.exists() {
        let cfg = EntityDrawingConfig::default();
        let cfg_str = toml::to_string_pretty(&cfg).expect("Serialize entity drawing config");
        let mut cfg_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&cfg_path)
            .expect("Failed to create entity drawing config");
        cfg_file
            .write_all(cfg_str.as_bytes())
            .expect("Failed to write entity drawing config");
        return Ok(cfg);
    }
    let mut cfg_file = OpenOptions::new()
        .read(true)
        .open(&cfg_path)
        .expect("Failed to open entity drawing config");
    let mut cfg_str = String::new();
    cfg_file
        .read_to_string(&mut cfg_str)
        .expect("Failed to read entity drawing config");

    let cfg = toml::from_str::<EntityDrawingConfig>(&cfg_str);
    if let Ok(cfg) = cfg {
        Ok(cfg)
    } else {
        Err(io::Error::new(io::ErrorKind::Other, cfg.err().unwrap()))
    }
}

pub fn load_entity_defs() -> io::Result<EditorEntities> {
    let cfg_path = util::get_config_dir()
        .expect("Failed to get config dir")
        .join("ent_defs.toml");
    if !cfg_path.exists() {
        let cfg = EditorEntities::default();
        let cfg_str = toml::to_string_pretty(&cfg).expect("Serialize entity defs config");
        let mut cfg_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&cfg_path)
            .expect("Failed to create entity defs config");
        cfg_file
            .write_all(cfg_str.as_bytes())
            .expect("Failed to write entity defs config");
        return Ok(cfg);
    }
    let mut cfg_file = OpenOptions::new()
        .read(true)
        .open(&cfg_path)
        .expect("Failed to open entity defs config");
    let mut cfg_str = String::new();
    cfg_file
        .read_to_string(&mut cfg_str)
        .expect("Failed to read entity defs config");

    match toml::from_str::<EditorEntities>(&cfg_str) {
        Ok(mut cfg) => {
            cfg.defs.sort();
            cfg.defs.dedup();
            Ok(cfg)
        }
        Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
    }
}

pub fn save(cfg: &EditorConfig) -> io::Result<String> {
    let old = load()?;
    if &old == cfg {
        return Ok("Not saving config".to_string());
    }
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
        Ok(_) => Ok(format!("Saved configuration to {}", cfg_path.display())),
        Err(e) => Err(e),
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
        "show_models" => {
            let value_u8: u8 = util::to_num(value);
            let res = value_u8 == 1;
            cfg.view.show.models = res;
            *view_config_rev = view_config_rev.wrapping_add(1);
            log_info!(console, "Set show models to {}", res);
        }
        "show_clip" => {
            let value_u8: u8 = util::to_num(value);
            let res = value_u8 == 1;
            cfg.view.show.clip_brushes = res;
            *view_config_rev = view_config_rev.wrapping_add(1);
            log_info!(console, "Set show clip brushes to {}", res);
        }
        "show_patches" => {
            let value_u8: u8 = util::to_num(value);
            let res = value_u8 == 1;
            cfg.view.show.patches = res;
            *view_config_rev = view_config_rev.wrapping_add(1);
            log_info!(console, "Set show patches to {}", res);
        }
        "show_convex" => {
            let value_u8: u8 = util::to_num(value);
            let res = value_u8 == 1;
            cfg.view.show.convex = res;
            *view_config_rev = view_config_rev.wrapping_add(1);
            log_info!(console, "Set show convex to {}", res);
        }
        "perf_tex_load_num" => {
            let value_u: usize = util::to_num(value);
            cfg.perf.tex_load_num = value_u;
            log_info!(console, "Change texture loading batch size to {}", value_u);
        }
        _ => (),
    }
}
