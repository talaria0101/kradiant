use std::{
    fs::OpenOptions, io::{Read, Write}, path::PathBuf
};

use crate::{ui::console::ConsoleLogger, util};
use num_traits::NumCast;
use serde::{Deserialize, Serialize};
use strum::VariantArray;

#[derive(Clone, Deserialize, Serialize, strum_macros::AsRefStr, strum_macros::VariantArray, PartialEq)]
pub enum RenderMode {
    /// No triangles
    None,
    Flat,
    Nearest,
    NearestMipmap,
    Linear,
    Bilinear,
    BilinearMipmap,
    Trilinear
}

impl RenderMode {
    pub fn all() -> [Self; 8]
    {
        [Self::None, Self::Flat, Self::Nearest, Self::NearestMipmap, Self::Linear, Self::Bilinear, Self::BilinearMipmap, Self::Trilinear]
    }
}


#[derive(Clone, Deserialize, Serialize)]
pub struct ViewConfig {
    pub grid_snap: bool,
    pub grid_minor_step: u8,
    pub fov: f32,
    pub wireframe: bool,
    pub rendermode: RenderMode,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct MiscConfig {
    pub theme: usize,
    pub recent_maps: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PathsConfig {
    pub main: PathBuf,
    pub texdir: PathBuf,
    pub scrdir: PathBuf,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct EditorConfig {
    // view
    pub view: ViewConfig,
    // misc
    pub misc: MiscConfig,
    // paths
    pub paths: PathsConfig,
}

impl EditorConfig {
    pub fn load() -> (Self, Option<String>) {
        let cfg_path = util::get_config_dir()
            .expect("Failed to get config dir")
            .join("prefs.toml");
        if !cfg_path.exists() {
            return (Self::default(), None);
        }
        let mut cfg_file = OpenOptions::new()
            .read(true)
            .open(&cfg_path)
            .expect("Failed to open config");
        let mut cfg_str = String::new();
        cfg_file
            .read_to_string(&mut cfg_str)
            .expect("Failed to read config");
        match toml::from_str::<Self>(&cfg_str) {
            Ok(cfg) => (cfg, None),
            Err(e) => (
                Self::default(),
                Some(format!(
                    "Failed to load config from {}: {e}",
                    cfg_path.display()
                )),
            ),
        }
    }

    pub fn save(&self) -> String {
        let cfg_str: String = toml::to_string(self).expect("Serialize config");
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

    pub fn update(&mut self, key: &str, value: impl NumCast, console: &mut ConsoleLogger) {
        match key {
            "grid_snap" => {
                let value_u8: u8 = util::to_num(value);
                let b = value_u8 == 1;
                self.view.grid_snap = b;
                log_info!(console, "Set grid snap to {}", self.view.grid_snap);
            }
            "grid_minor_step" => {
                self.view.grid_minor_step = util::to_num(value);
                log_info!(console, "Set grid step to {}", self.view.grid_minor_step);
            }
            "view3d_fov" => {
                self.view.fov = util::to_num(value);
                log_info!(console, "Set 3d view fov to {}", self.view.fov);
                //
            }
            "active_theme" => {
                self.misc.theme = util::to_num(value);
                log_info!(console, "Set active theme index to {}", self.misc.theme);
            }
            "wireframe" => {
                let value_u8: u8 = util::to_num(value);
                let b = value_u8 == 1;
                self.view.wireframe = b;
                log_info!(console, "Set wireframe to {}", self.view.wireframe);
            }
            "rendermode" => {
                let value_usize: usize = util::to_num(value);
                let mode = RenderMode::VARIANTS.get(value_usize).unwrap_or(&RenderMode::Flat);
                self.view.rendermode = mode.to_owned();
                log_info!(console, "Set rendermode to {}", self.view.rendermode.as_ref());
            }
            _ => (),
        }
    }
}

impl Default for EditorConfig {
    fn default() -> Self {
        let game_main = PathBuf::from(".");
        let game_texdir = game_main.join("textures");
        let game_scrdir = game_main.join("scripts");
        Self {
            view: ViewConfig {
                grid_snap: true,
                grid_minor_step: 4,
                fov: 80.0,
                wireframe: true,
                rendermode: RenderMode::Flat,
            },
            misc: MiscConfig {
                recent_maps: vec![],
                theme: 0,
            },
            paths: PathsConfig {
                main: game_main,
                texdir: game_texdir,
                scrdir: game_scrdir,
            },
        }
    }
}

/*
impl From<Ini> for EditorConfig {
    fn from(value: Ini) -> Self {
        // view
        let g_sn = value.get_from(Some("view"), "grid_snap").unwrap_or("true");
        let g_sz = value.get_from(Some("view"), "grid_size").unwrap_or("4");
        let fov = value.get_from(Some("view"), "3d_fov").unwrap_or("80");

        // misc
        let theme = value.get_from(Some("misc"), "active_theme").unwrap_or("0");

        //println!("{:#?}", value.get_from(Some("paths"), "main"));

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
*/
