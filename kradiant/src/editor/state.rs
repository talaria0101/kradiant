use crate::editor::selection::{FaceSelection, PatchVertexSelection};
use crate::editor::undo::UndoRedo;
use crate::map::Map;

#[derive(Debug)]
pub struct EditorState {
    pub map_path: String,
    pub map_revision: u64,
    pub map: Option<Map>,
    pub map_load_count: u64,

    pub selected_entity: Option<usize>,
    pub selected_brushes: Vec<(usize, usize)>,
    pub selected_faces: Vec<FaceSelection>,
    pub selected_patch_vertices: Vec<PatchVertexSelection>,

    pub new_prop_key: String,
    pub new_prop_val: String,

    pub edit_faces: bool,
    pub edit_edges: bool,
    pub edit_vertices: bool,

    pub undo: UndoRedo,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            map_path: String::new(),
            map_revision: 0,
            map: Some(Map::default()),
            map_load_count: 0,
            selected_entity: None,
            selected_brushes: Vec::new(),
            selected_faces: Vec::new(),
            selected_patch_vertices: Vec::new(),
            new_prop_key: String::new(),
            new_prop_val: String::new(),
            edit_faces: false,
            edit_edges: false,
            edit_vertices: false,
            undo: UndoRedo::default(),
        }
    }
}

impl EditorState {
    pub fn bump_revision(&mut self) {
        self.map_revision = self.map_revision.wrapping_add(1);
    }

    pub fn clear_selection(&mut self) {
        self.selected_entity = None;
        self.selected_brushes.clear();
        self.selected_faces.clear();
        self.selected_patch_vertices.clear();
    }
}

