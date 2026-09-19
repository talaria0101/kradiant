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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Show {
    pub models: bool,
    pub clip_brushes: bool,
    pub portal_brushes: bool,
    pub hint_brushes: bool,
    pub patches: bool,
    pub convex: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self {
            models: true,
            clip_brushes: true,
            portal_brushes: true,
            hint_brushes: true,
            patches: true,
            convex: true,
        }
    }
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
        Self::SolidBox
    }
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize, PartialEq)]
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
    pub arrow_length: f32,
    pub anchor: EntityDrawAnchor,
    /// Small axis-aligned box drawn at the model root bone (entity origin)
    /// to help placement of entities with models.
    pub origin_box_size: [f32; 3],
    pub show_origin_box: bool,
}

impl Default for EntityDrawStyle {
    fn default() -> Self {
        Self {
            kind: EntityDrawKind::SolidBox,
            size: [32.0, 32.0, 32.0],
            color: [1.0, 1.0, 1.0, 1.0],
            show_arrow: true,
            arrow_length: 20.0,
            anchor: EntityDrawAnchor::Center,
            origin_box_size: [4.0, 4.0, 4.0],
            show_origin_box: true,
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
            .find(|rule| {
                if rule.classname.starts_with("*") {
                    if classname.ends_with(&rule.classname[1..]) {
                        return true;
                    }
                }
                if rule.classname.ends_with("*") {
                    if classname.starts_with(&rule.classname[..rule.classname.len() - 1]) {
                        return true;
                    }
                }
                if rule.classname == classname {
                    return true;
                } else {
                    return false;
                }
            })
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
                arrow_length: 0.0,
                anchor: EntityDrawAnchor::Center,
                origin_box_size: [4.0, 4.0, 4.0],
                show_origin_box: true,
            },
            default_without_model: EntityDrawStyle {
                kind: EntityDrawKind::Box,
                size: [32.0, 32.0, 32.0],
                color: [0.239, 0.239, 0.8, 1.0],
                show_arrow: false,
                arrow_length: 0.0,
                anchor: EntityDrawAnchor::Center,
                origin_box_size: [4.0, 4.0, 4.0],
                show_origin_box: true,
            },
            rules: vec![
                EntityDrawRule {
                    classname: "mp_deathmatch_spawn".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 72.0],
                        color: [0.8, 0.2, 0.0, 1.0],
                        show_arrow: false,
                        arrow_length: 0.0,
                        anchor: EntityDrawAnchor::Base,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                EntityDrawRule {
                    classname: "mp_teamdeathmatch_spawn".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 72.0],
                        color: [0.0, 0.0, 0.8, 1.0],
                        show_arrow: false,
                        arrow_length: 0.0,
                        anchor: EntityDrawAnchor::Base,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                EntityDrawRule {
                    classname: "mp_searchanddestroy_spawn_allied".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::ModelBounds,
                        size: [32.0, 32.0, 32.0],
                        color: [0.0, 0.8, 0.0, 1.0],
                        show_arrow: false,
                        arrow_length: 0.0,
                        anchor: EntityDrawAnchor::Center,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                EntityDrawRule {
                    classname: "mp_searchanddestroy_spawn_axis".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::ModelBounds,
                        size: [32.0, 32.0, 32.0],
                        color: [0.8, 0.0, 0.0, 1.0],
                        show_arrow: false,
                        arrow_length: 0.0,
                        anchor: EntityDrawAnchor::Center,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                EntityDrawRule {
                    classname: "mp_retrieval_spawn_allied".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 72.0],
                        color: [0.2, 0.0, 0.8, 1.0],
                        show_arrow: false,
                        arrow_length: 0.0,
                        anchor: EntityDrawAnchor::Base,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                EntityDrawRule {
                    classname: "mp_retrieval_spawn_axis".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 72.0],
                        color: [0.8, 0.0, 0.2, 1.0],
                        show_arrow: false,
                        arrow_length: 0.0,
                        anchor: EntityDrawAnchor::Base,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                EntityDrawRule {
                    classname: "mp_searchanddestroy_intermission".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 32.0],
                        color: [0.8, 0.0, 0.8, 1.0],
                        show_arrow: true,
                        arrow_length: 20.0,
                        anchor: EntityDrawAnchor::Base,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                EntityDrawRule {
                    classname: "mp_deathmatch_intermission".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 32.0],
                        color: [0.8, 0.0, 0.8, 1.0],
                        show_arrow: true,
                        arrow_length: 20.0,
                        anchor: EntityDrawAnchor::Base,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                EntityDrawRule {
                    classname: "mp_teamdeathmatch_intermission".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 32.0],
                        color: [0.8, 0.0, 0.8, 1.0],
                        show_arrow: true,
                        arrow_length: 20.0,
                        anchor: EntityDrawAnchor::Base,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                EntityDrawRule {
                    classname: "mp_retrieval_intermission".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::SolidBox,
                        size: [32.0, 32.0, 32.0],
                        color: [0.0, 0.2, 0.8, 1.0],
                        show_arrow: true,
                        arrow_length: 20.0,
                        anchor: EntityDrawAnchor::Base,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                EntityDrawRule {
                    classname: "misc_model".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::Box,
                        size: [4.0, 4.0, 4.0],
                        color: [0.8, 0.0, 0.8, 1.0],
                        show_arrow: true,
                        arrow_length: 20.0,
                        anchor: EntityDrawAnchor::Center,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                // Triggers have brushes, they don't need proxy representation
                EntityDrawRule {
                    classname: "trigger*".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::Hidden,
                        size: [4.0, 4.0, 4.0],
                        color: [0.5, 0.5, 0.5, 0.0],
                        show_arrow: false,
                        arrow_length: 0.0,
                        anchor: EntityDrawAnchor::Base,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
                    },
                },
                EntityDrawRule {
                    classname: "item*".to_string(),
                    style: EntityDrawStyle {
                        kind: EntityDrawKind::Box,
                        size: [16.0, 16.0, 16.0],
                        color: [0.4, 0.4, 0.8, 1.0],
                        show_arrow: true,
                        arrow_length: 20.0,
                        anchor: EntityDrawAnchor::Base,
                        origin_box_size: [4.0, 4.0, 4.0],
                        show_origin_box: true,
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
            perf: Performance { tex_load_num: 16 },
        }
    }
}

#[derive(Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, Clone, Debug)]
pub struct EntityDef {
    pub class: String,
    pub props: Vec<(String, String)>,
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
pub struct EditorEntities {
    pub defs: Vec<EntityDef>,
}

impl Default for EditorEntities {
    fn default() -> Self {
        let mut defs: Vec<EntityDef> = Vec::new();

        defs.push(EntityDef {
            class: String::from("mp_deathmatch_spawn"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("mp_deathmatch_intermission"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("mp_teamdeathmatch_spawn"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("mp_teamdeathmatch_intermission"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("mp_searchanddestroy_spawn_allied"),
            props: vec![(String::from("model"), String::from("xmodel/airborne"))],
        });
        defs.push(EntityDef {
            class: String::from("mp_searchanddestroy_spawn_axis"),
            props: vec![(
                String::from("model"),
                String::from("xmodel/wehrmacht_soldier"),
            )],
        });
        defs.push(EntityDef {
            class: String::from("mp_searchanddestroy_intermission"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("mp_retrieval_spawn_allied"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("mp_retrieval_spawn_axis"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("mp_retrieval_intermission"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("mp_retrieval_objective"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("mp_target_location"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("item_ammo_stielhandgranate_closed"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("item_ammo_stielhandgranate_open"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("item_health"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("item_health_large"),
            props: Vec::new(),
        });
        defs.push(EntityDef {
            class: String::from("item_health_small"),
            props: Vec::new(),
        });

        defs.sort();

        Self { defs }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_uses_loaded_model_state_not_property_key() {
        // Regression: has_model must be entity.model.is_some(), NOT
        // entity.properties.contains_key("model"). Otherwise a failed
        // model load would still resolve default_with_model.
        let cfg = EntityDrawingConfig {
            default_with_model: EntityDrawStyle {
                kind: EntityDrawKind::Box,
                size: [64.0, 64.0, 64.0],
                ..Default::default()
            },
            default_without_model: EntityDrawStyle {
                kind: EntityDrawKind::SolidBox,
                size: [8.0, 8.0, 8.0],
                ..Default::default()
            },
            rules: vec![],
        };

        // No model property, no loaded model → default_without_model
        let s = cfg.resolve("info_player_start", false);
        assert_eq!(s.kind, EntityDrawKind::SolidBox);
        assert_eq!(s.size, [8.0, 8.0, 8.0]);

        // Model property exists but model NOT loaded (failed load) →
        // has_model=false → default_without_model
        let s = cfg.resolve("info_player_start", false);
        assert_eq!(s.kind, EntityDrawKind::SolidBox);

        // Model property exists and model IS loaded → default_with_model
        let s = cfg.resolve("info_player_start", true);
        assert_eq!(s.kind, EntityDrawKind::Box);
        assert_eq!(s.size, [64.0, 64.0, 64.0]);
    }
}
