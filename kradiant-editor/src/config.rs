use crate::{ui::console::ConsoleLogger, util};
use ini::Ini;
use num_traits::NumCast;

#[derive(Clone)]
pub struct EditorConfig {
    pub grid_snap: bool,
    pub grid_minor_step: u8,
    pub view3d_fov: f32,
    pub active_theme: usize,
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
        Self {
            grid_snap: true,
            grid_minor_step: 4,
            view3d_fov: 80.0,
            active_theme: 0,
        }
    }
}

impl From<Ini> for EditorConfig {
    fn from(value: Ini) -> Self {
        let g_sn = value.get_from(Some("view"), "grid_snap").unwrap_or("true");
        let g_sz = value.get_from(Some("view"), "grid_size").unwrap_or("4");
        let fov = value.get_from(Some("view"), "3d_fov").unwrap_or("80");
        let theme = value.get_from(Some("misc"), "active_theme").unwrap_or("0");

        Self {
            grid_snap: g_sn.parse().unwrap(),
            grid_minor_step: g_sz.parse().unwrap(),
            view3d_fov: fov.parse().unwrap(),
            active_theme: theme.parse().unwrap(),
        }
    }
}

impl From<EditorConfig> for Ini {
    fn from(cfg: EditorConfig) -> Self {
        let mut conf = Ini::new();
        conf.with_section(Some("view"))
            .set("grid_snap", cfg.grid_snap.to_string())
            .set("grid_size", cfg.grid_minor_step.to_string())
            .set("3d_fov", cfg.view3d_fov.to_string());

        conf.with_section(Some("misc"))
            .set("active_theme", cfg.active_theme.to_string());

        conf
    }
}
