use std::io;

use crate::{ui::console::ConsoleLogger, util};
pub use kradiant::editor::config::RenderMode;
use kradiant::editor::config::{EditorConfig, EditorEntities, EntityDrawingConfig};
use num_traits::NumCast;
use strum::VariantArray;

pub fn load() -> io::Result<EditorConfig> {
    let cfg_str = match util::read_cfg("prefs.toml") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading config: {e}");
            return Ok(EditorConfig::default());
        }
    };

    match toml::from_str(&cfg_str) {
        Ok(c) => Ok(c),
        Err(e) => Err(io::Error::other(e))
    }
}

pub fn load_entity_drawing() -> io::Result<EntityDrawingConfig> {
    let cfg_str = match util::read_cfg("ent_drawing.toml") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading config: {e}");
            String::new()
        }
    };

    match toml::from_str(&cfg_str) {
        Ok(c) => return Ok(c),
        Err(e) => {
            eprintln!("Error parsing ent_drawing.toml: {e}");
        }
    }

    let def = EntityDrawingConfig::default();
    if let Err(e) = util::write_cfg("ent_defs.toml", &toml::to_string(&def).unwrap_or_default()) {
        eprintln!("Failed to write default ent_defs.toml: {e}");
    }

    Ok(def)
}

pub fn load_entity_defs() -> io::Result<EditorEntities> {
    let cfg_str = match util::read_cfg("ent_defs.toml") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading ent_defs.toml: {e}");
            String::new()
        }
    };

    match toml::from_str::<EditorEntities>(&cfg_str) {
        Ok(mut cfg) => {
            cfg.defs.sort();
            cfg.defs.dedup();
            return Ok(cfg);
        }
        Err(e) => eprintln!("Error parsing ent_defs.toml: {e}"),
    };

    let def = EditorEntities::default();
    if let Err(e) = util::write_cfg("ent_defs.toml", &toml::to_string(&def).unwrap_or_default()) {
        eprintln!("Failed to write default ent_defs.toml: {e}");
    }

    Ok(def)
}

pub fn save(cfg: &EditorConfig) -> io::Result<String> {
    let old = load().unwrap_or(EditorConfig::default());
    if &old == cfg {
        return Ok("Not saving config".to_string());
    }
    let cfg_str: String = toml::to_string(cfg).expect("Serialize config");

    match util::write_cfg("prefs.toml", &cfg_str) {
        Ok(_) => Ok(format!("Saved configuration")),
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
        "show_portal" => {
            let value_u8: u8 = util::to_num(value);
            let res = value_u8 == 1;
            cfg.view.show.portal_brushes = res;
            *view_config_rev = view_config_rev.wrapping_add(1);
            log_info!(console, "Set show portal brushes to {}", res);
        }
        "show_hint" => {
            let value_u8: u8 = util::to_num(value);
            let res = value_u8 == 1;
            cfg.view.show.hint_brushes = res;
            *view_config_rev = view_config_rev.wrapping_add(1);
            log_info!(console, "Set show hint brushes to {}", res);
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
