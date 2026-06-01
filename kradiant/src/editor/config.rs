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

#[derive(Clone, Debug, Deserialize, Serialize, Default, PartialEq)]
pub struct Show {
    pub models: bool,
    pub clip_brushes: bool,
    pub patches: bool,
    pub convex: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EntityDrawKind {
    Box,
    SolidBox,
    ModelBounds,
    ModelWireframe,
    Hidden,
}

impl Default for EntityDrawKind {
    fn default() -> Self {
        Self::ModelBounds
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EntityDrawAnchor {
    Center,
    Base,
}

impl Default for EntityDrawAnchor {
    fn default() -> Self {
        Self::Center
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct EntityDrawStyle {
    pub kind: EntityDrawKind,
    pub size: [f32; 3],
    pub color: [f32; 4],
    pub show_arrow: bool,
    pub anchor: EntityDrawAnchor,
}

impl Default for EntityDrawStyle {
    fn default() -> Self {
        Self {
            kind: EntityDrawKind::ModelBounds,
            size: [32.0, 32.0, 32.0],
            color: [1.0, 1.0, 1.0, 1.0],
            show_arrow: true,
            anchor: EntityDrawAnchor::Center,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct EntityDrawRule {
    pub classname: String,
    pub style: EntityDrawStyle,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct EntityDrawingConfig {
    pub default_with_model: EntityDrawStyle,
    pub default_without_model: EntityDrawStyle,
    pub rules: Vec<EntityDrawRule>,
}

impl Default for EntityDrawingConfig {
    fn default() -> Self {
        Self::cod_default()
    }
}

impl EntityDrawingConfig {
    pub fn resolve(&self, classname: &str, has_model: bool) -> EntityDrawStyle {
        self.rules
            .iter()
            .find(|rule| rule.classname == classname)
            .map(|rule| rule.style.clone())
            .unwrap_or_else(|| {
                if has_model {
                    self.default_with_model.clone()
                } else {
                    self.default_without_model.clone()
                }
            })
    }

    pub fn cod_default() -> Self {
        Self {
            default_with_model: EntityDrawStyle {
                kind: EntityDrawKind::Box,
                size: [32.0, 32.0, 32.0],
                color: [0.239, 0.239, 0.8, 1.0],
                show_arrow: false,
                anchor: EntityDrawAnchor::Center,
            },
            default_without_model: EntityDrawStyle {
                kind: EntityDrawKind::Box,
                size: [32.0, 32.0, 32.0],
                color: [0.239, 0.239, 0.8, 1.0],
                show_arrow: false,
                anchor: EntityDrawAnchor::Center,
            },
            rules: vec![
                EntityDrawRule {
                    classname: "mp_deathmatch_spawn".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 72.0],
                        color: [0.8, 0.2, 0.0, 1.0],
                        show_arrow: false,
                        anchor: EntityDrawAnchor::Base,
                    },
                },
                EntityDrawRule {
                    classname: "mp_teamdeathmatch_spawn".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 72.0],
                        color: [0.0, 0.0, 0.8, 1.0],
                        show_arrow: false,
                        anchor: EntityDrawAnchor::Base,
                    },
                },
                EntityDrawRule {
                    classname: "mp_searchanddestroy_spawn_allied".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::ModelBounds,
                        size: [32.0, 32.0, 32.0],
                        color: [0.0, 0.8, 0.0, 1.0],
                        show_arrow: false,
                        anchor: EntityDrawAnchor::Center,
                    },
                },
                EntityDrawRule {
                    classname: "mp_searchanddestroy_spawn_axis".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::ModelBounds,
                        size: [32.0, 32.0, 32.0],
                        color: [0.8, 0.0, 0.0, 1.0],
                        show_arrow: false,
                        anchor: EntityDrawAnchor::Center,
                    },
                },
                EntityDrawRule {
                    classname: "mp_retrieval_spawn_allied".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 72.0],
                        color: [0.2, 0.0, 0.8, 1.0],
                        show_arrow: false,
                        anchor: EntityDrawAnchor::Base,
                    },
                },
                EntityDrawRule {
                    classname: "mp_retrieval_spawn_axis".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 72.0],
                        color: [0.8, 0.0, 0.2, 1.0],
                        show_arrow: false,
                        anchor: EntityDrawAnchor::Base,
                    },
                },
                EntityDrawRule {
                    classname: "mp_searchanddestroy_intermission".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 32.0],
                        color: [0.8, 0.0, 0.8, 1.0],
                        show_arrow: false,
                        anchor: EntityDrawAnchor::Base,
                    },
                },
                EntityDrawRule {
                    classname: "mp_deathmatch_intermission".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 32.0],
                        color: [0.8, 0.0, 0.8, 1.0],
                        show_arrow: false,
                        anchor: EntityDrawAnchor::Base,
                    },
                },
                EntityDrawRule {
                    classname: "mp_teamdeathmatch_intermission".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 32.0],
                        color: [0.8, 0.0, 0.8, 1.0],
                        show_arrow: false,
                        anchor: EntityDrawAnchor::Base,
                    },
                },
                EntityDrawRule {
                    classname: "mp_retrieval_intermission".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 32.0],
                        color: [0.0, 0.2, 0.8, 1.0],
                        show_arrow: false,
                        anchor: EntityDrawAnchor::Base,
                    },
                },
                EntityDrawRule {
                    classname: "misc_model".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::Box,
                        size: [4.0, 4.0, 4.0],
                        color: [0.8, 0.0, 0.8, 1.0],
                        show_arrow: false,
                        anchor: EntityDrawAnchor::Center,
                    },
                },
            ],
        }
    }
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
pub struct ViewConfig {
    pub grid_snap: bool,
    pub grid_minor_step: u8,
    pub fov: f32,
    pub wireframe: bool,
    pub rendermode: RenderMode,
    pub show: Show,
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
pub struct MiscConfig {
    pub theme: usize,
    pub recent_maps: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
pub struct PathsConfig {
    pub main: PathBuf,
    pub texdir: PathBuf,
    pub scrdir: PathBuf,
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
pub struct Performance {
    pub tex_load_num: usize,
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
pub struct EditorConfig {
    pub view: ViewConfig,
    pub misc: MiscConfig,
    pub paths: PathsConfig,
    pub perf: Performance,
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
                show: Show::default(),
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
            perf: Performance {
                tex_load_num: 16
            }
        }
    }
}
