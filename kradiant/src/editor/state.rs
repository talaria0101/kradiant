use crate::assets::AssetDb;
use crate::editor::config::EditorConfig;
use crate::editor::config::EditorEntities;
use crate::editor::config::EntityDrawingConfig;
use crate::editor::selection::{EdgeSelection, FaceSelection, PatchVertexSelection};
use crate::editor::theme::EditorPalette;
use crate::editor::undo::UndoRedo;
use crate::editor::viewport::{StretchMode, View2DState, View3DState};
use crate::map::Map;
use crate::render::TextureRegistry;
use crate::shader::ShaderDb;
use std::collections::HashSet;

#[derive(Debug)]
pub struct EditorState {
    pub map_path: String,
    pub map_revision: u64,
    pub map: Option<Map>,
    pub map_load_count: u64,
    /// True when the map has been modified since the last save or load.
    pub dirty: bool,

    pub selected_entities: Vec<usize>,
    pub selected_brushes: Vec<(usize, usize)>,
    pub selected_faces: Vec<FaceSelection>,
    pub selected_edges: Vec<EdgeSelection>,
    pub selected_patch_vertices: Vec<PatchVertexSelection>,

    pub new_prop_key: String,
    pub new_prop_val: String,

    pub edit_faces: bool,
    pub edit_edges: bool,
    pub edit_vertices: bool,

    pub undo: UndoRedo,

    // UI-agnostic viewport and environment state
    pub view2d: View2DState,
    pub view3d: View3DState,
    pub config: EditorConfig,
    pub entity_drawing: EntityDrawingConfig,
    pub entity_defs: EditorEntities,
    pub palette: EditorPalette,
    pub view_config_rev: u64,
    pub tex_registry: TextureRegistry,
    pub shader_db: Option<ShaderDb>,

    // Editing modes
    pub stretch_mode: StretchMode,
    pub selection_rgba: [f32; 4],
    pub model_asset_db: Option<AssetDb>,
    /// Model names that failed to load; avoids retrying every frame.
    pub failed_models: HashSet<String>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            map_path: String::new(),
            map_revision: 0,
            map: Some(Map::default()),
            map_load_count: 0,
            dirty: false,
            selected_entities: Vec::new(),
            selected_brushes: Vec::new(),
            selected_faces: Vec::new(),
            selected_edges: Vec::new(),
            selected_patch_vertices: Vec::new(),
            new_prop_key: String::new(),
            new_prop_val: String::new(),
            edit_faces: false,
            edit_edges: false,
            edit_vertices: false,
            undo: UndoRedo::default(),
            view2d: View2DState::default(),
            view3d: View3DState::default(),
            config: EditorConfig::default(),
            entity_drawing: EntityDrawingConfig::default(),
            entity_defs: EditorEntities::default(),
            palette: EditorPalette::default(),
            view_config_rev: 0,
            tex_registry: TextureRegistry::default(),
            shader_db: None,
            stretch_mode: StretchMode::default(),
            selection_rgba: [0.3, 0.6, 1.0, 1.0],
            model_asset_db: None,
            failed_models: HashSet::new(),
        }
    }
}

impl EditorState {
    pub fn bump_revision(&mut self) {
        self.map_revision = self.map_revision.wrapping_add(1);
        self.dirty = true;
    }

    pub fn clear_selection(&mut self) {
        self.selected_entities.clear();
        self.selected_brushes.clear();
        self.selected_faces.clear();
        self.selected_edges.clear();
        self.selected_patch_vertices.clear();
    }
}
