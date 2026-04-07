use std::path::PathBuf;

use crate::{ui::console::ConsoleLogger, util};
use ini::Ini;
use num_traits::NumCast;

#[derive(Clone)]
pub struct EditorConfig {
    // view
    pub grid_snap: bool,
    pub grid_minor_step: u8,
    pub view3d_fov: f32,
    // misc
    pub active_theme: usize,
    // paths
    pub game_main: PathBuf,
    pub game_texdir: PathBuf,
    pub game_scrdir: PathBuf,
}

impl EditorConfig {
    pub fn load() -> Result<Self, ini::Error> {
        let cfg_file = util::get_config_dir()?.join("prefs.ini");
        Ini::load_from_file(&cfg_file).map(Self::from)
    }

    pub fn save(cfg: Self) -> String {
        let cfg_file = util::get_config_dir().unwrap().join("prefs.ini");
        match Ini::write_to_file(&cfg.into(), &cfg_file) {
            Ok(_) => format!("Saved configuration to {}", cfg_file.display()),
            Err(e) => format!(
                "Error saving configuration to {}: {}",
                cfg_file.display(),
                e
            ),
        }
    }

    pub fn update(&mut self, key: &str, value: impl NumCast, console: &mut ConsoleLogger) {
        match key {
            "grid_snap" => {
                let value_u8: u8 = util::to_num(value);
                let b = value_u8 == 1;
                self.grid_snap = b;
                log_info!(console, "Set grid snap to {}", self.grid_snap);
            }
            "grid_minor_step" => {
                self.grid_minor_step = util::to_num(value);
                log_info!(console, "Set grid step to {}", self.grid_minor_step);
            }
            "view3d_fov" => {
                self.view3d_fov = util::to_num(value);
                log_info!(console, "Set 3d view fov to {}", self.view3d_fov);
                //
            }
            "active_theme" => {
                self.active_theme = util::to_num(value);
                log_info!(console, "Set active theme index to {}", self.active_theme);
            }
            _ => (),
        }
    }
}

impl Default for EditorConfig {
    fn default() -> Self {
        let game_main = PathBuf::new();
        let game_texdir = game_main.join("textures");
        let game_scrdir = game_main.join("scripts");
        Self {
            grid_snap: true,
            grid_minor_step: 4,
            view3d_fov: 80.0,
            active_theme: 0,
            game_main,
            game_texdir,
            game_scrdir,
        }
    }
}

impl From<Ini> for EditorConfig {
    fn from(value: Ini) -> Self {
        // view
        let g_sn = value.get_from(Some("view"), "grid_snap").unwrap_or("true");
        let g_sz = value.get_from(Some("view"), "grid_size").unwrap_or("4");
        let fov = value.get_from(Some("view"), "3d_fov").unwrap_or("80");

        // misc
        let theme = value.get_from(Some("misc"), "active_theme").unwrap_or("0");

        // paths
        let game_main = value
            .get_from(Some("paths"), "main")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let game_texdir = value
            .get_from(Some("paths"), "texture_dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./textures"));
        let game_scrdir = value
            .get_from(Some("paths"), "script_dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./scripts"));

        Self {
            grid_snap: g_sn.parse().unwrap(),
            grid_minor_step: g_sz.parse().unwrap(),
            view3d_fov: fov.parse().unwrap(),
            active_theme: theme.parse().unwrap(),
            game_main,
            game_texdir,
            game_scrdir,
        }
    }
}

impl From<EditorConfig> for Ini {
    fn from(cfg: EditorConfig) -> Self {
        let mut conf = Ini::new();

        // view
        conf.with_section(Some("view"))
            .set("grid_snap", cfg.grid_snap.to_string())
            .set("grid_size", cfg.grid_minor_step.to_string())
            .set("3d_fov", cfg.view3d_fov.to_string());

        // misc
        conf.with_section(Some("misc"))
            .set("active_theme", cfg.active_theme.to_string());

        // paths
        conf.with_section(Some("paths"))
            .set("main", cfg.game_main.to_string_lossy().to_string())
            .set("texture_dir", cfg.game_texdir.to_string_lossy().to_string())
            .set("script_dir", cfg.game_scrdir.to_string_lossy().to_string());

        conf
    }
}
