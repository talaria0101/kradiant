use crate::util;
use ini::Ini;

#[derive(Clone)]
pub struct EditorConfig {
    pub grid_minor_step: u8,
    pub view3d_fov: f32,
}

impl EditorConfig {
    pub fn load() -> Result<Self, ini::Error> {
        let cfg_dir = util::get_config_dir().unwrap();
        let cfg_file = cfg_dir.join("prefs.ini");

        match Ini::load_from_file(&cfg_file) {
            Ok(c) => Ok(c.into()),
            Err(e) => Err(e),
        }
    }

    pub fn save(cfg: Self) -> String {
        let cfg_dir = util::get_config_dir().unwrap();
        let cfg_file = cfg_dir.join("prefs.ini");

        match Ini::write_to_file(&cfg.into(), &cfg_file) {
            Ok(_) => format!("Saved configuration to {}", cfg_file.display()),
            Err(e) => format!(
                "Error saving configuration to {}: {}",
                cfg_file.display(),
                e.to_string()
            ),
        }
    }
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            grid_minor_step: 4,
            view3d_fov: 80.0,
        }
    }
}

impl From<Ini> for EditorConfig {
    fn from(value: Ini) -> Self {
        let g_s = value.get_from(Some("view"), "grid_size").unwrap_or("4");
        let fov = value.get_from(Some("view"), "3d_fov").unwrap_or("80");

        Self {
            grid_minor_step: g_s.parse().unwrap(),
            view3d_fov: fov.parse().unwrap(),
        }
    }
}

impl Into<Ini> for EditorConfig {
    fn into(self) -> Ini {
        let mut conf = Ini::new();
        conf.with_section(Some("view"))
            .set("grid_size", self.grid_minor_step.to_string())
            .set("3d_fov", self.view3d_fov.to_string());

        conf
    }
}
