use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(
    Debug,
    Clone,
    Deserialize,
    Serialize,
    strum_macros::AsRefStr,
    strum_macros::VariantArray,
    PartialEq,
)]
pub enum RenderMode {
    /// No triangles
    None,
    Flat,
    Nearest,
    NearestMipmap,
    Linear,
    Bilinear,
    BilinearMipmap,
    Trilinear,
}

impl RenderMode {
    pub fn all() -> [Self; 8] {
        [
            Self::None,
            Self::Flat,
            Self::Nearest,
            Self::NearestMipmap,
            Self::Linear,
            Self::Bilinear,
            Self::BilinearMipmap,
            Self::Trilinear,
        ]
    }
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct ViewConfig {
    pub grid_snap: bool,
    pub grid_minor_step: u8,
    pub fov: f32,
    pub wireframe: bool,
    pub rendermode: RenderMode,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct MiscConfig {
    pub theme: usize,
    pub recent_maps: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct PathsConfig {
    pub main: PathBuf,
    pub texdir: PathBuf,
    pub scrdir: PathBuf,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct EditorConfig {
    pub view: ViewConfig,
    pub misc: MiscConfig,
    pub paths: PathsConfig,
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
