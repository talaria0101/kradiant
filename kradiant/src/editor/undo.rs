use crate::editor::selection::{FaceSelection, PatchVertexSelection};
use crate::map::Map;

#[derive(Debug)]
struct Snapshot {
    map: Option<Map>,
    selected_brushes: Vec<(usize, usize)>,
    selected_faces: Vec<FaceSelection>,
    selected_patch_vertices: Vec<PatchVertexSelection>,
    selected_entity: Option<usize>,
}

#[derive(Debug)]
struct Entry {
    label: String,
    snapshot: Snapshot,
}

#[derive(Debug)]
pub struct UndoRedo {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    max_entries: usize,
}

impl Default for UndoRedo {
    fn default() -> Self {
        Self::new(64)
    }
}

impl UndoRedo {
    pub fn new(max_entries: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            max_entries: max_entries.max(1),
        }
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn push(
        &mut self,
        label: impl Into<String>,
        map: &Option<Map>,
        selected_brushes: &[(usize, usize)],
        selected_faces: &[FaceSelection],
        selected_patch_vertices: &[PatchVertexSelection],
        selected_entity: &Option<usize>,
    ) {
        self.redo.clear();

        self.undo.push(Entry {
            label: label.into(),
            snapshot: Snapshot {
                map: map.clone(),
                selected_brushes: selected_brushes.to_vec(),
                selected_faces: selected_faces.to_vec(),
                selected_patch_vertices: selected_patch_vertices.to_vec(),
                selected_entity: *selected_entity,
            },
        });

        if self.undo.len() > self.max_entries {
            let extra = self.undo.len() - self.max_entries;
            self.undo.drain(0..extra);
        }
    }

    pub fn push_map(
        &mut self,
        label: impl Into<String>,
        map: &Map,
        selected_brushes: &[(usize, usize)],
        selected_faces: &[FaceSelection],
        selected_patch_vertices: &[PatchVertexSelection],
        selected_entity: &Option<usize>,
    ) {
        self.redo.clear();
        self.undo.push(Entry {
            label: label.into(),
            snapshot: Snapshot {
                map: Some(map.clone()),
                selected_brushes: selected_brushes.to_vec(),
                selected_faces: selected_faces.to_vec(),
                selected_patch_vertices: selected_patch_vertices.to_vec(),
                selected_entity: *selected_entity,
            },
        });
        if self.undo.len() > self.max_entries {
            let extra = self.undo.len() - self.max_entries;
            self.undo.drain(0..extra);
        }
    }

    pub fn undo(
        &mut self,
        map: &mut Option<Map>,
        selected_brushes: &mut Vec<(usize, usize)>,
        selected_faces: &mut Vec<FaceSelection>,
        selected_patch_vertices: &mut Vec<PatchVertexSelection>,
        selected_entity: &mut Option<usize>,
        map_revision: &mut u64,
    ) -> Option<String> {
        let entry = self.undo.pop()?;

        // Move current state to redo.
        let current = Snapshot {
            map: map.take(),
            selected_brushes: std::mem::take(selected_brushes),
            selected_faces: std::mem::take(selected_faces),
            selected_patch_vertices: std::mem::take(selected_patch_vertices),
            selected_entity: selected_entity.take(),
        };
        self.redo.push(Entry {
            label: entry.label.clone(),
            snapshot: current,
        });

        *map = entry.snapshot.map;
        *selected_brushes = entry.snapshot.selected_brushes;
        *selected_faces = entry.snapshot.selected_faces;
        *selected_patch_vertices = entry.snapshot.selected_patch_vertices;
        *selected_entity = entry.snapshot.selected_entity;
        *map_revision = map_revision.wrapping_add(1);
        Some(entry.label)
    }

    pub fn redo(
        &mut self,
        map: &mut Option<Map>,
        selected_brushes: &mut Vec<(usize, usize)>,
        selected_faces: &mut Vec<FaceSelection>,
        selected_patch_vertices: &mut Vec<PatchVertexSelection>,
        selected_entity: &mut Option<usize>,
        map_revision: &mut u64,
    ) -> Option<String> {
        let entry = self.redo.pop()?;

        // Move current state to undo.
        let current = Snapshot {
            map: map.take(),
            selected_brushes: std::mem::take(selected_brushes),
            selected_faces: std::mem::take(selected_faces),
            selected_patch_vertices: std::mem::take(selected_patch_vertices),
            selected_entity: selected_entity.take(),
        };
        self.undo.push(Entry {
            label: entry.label.clone(),
            snapshot: current,
        });

        *map = entry.snapshot.map;
        *selected_brushes = entry.snapshot.selected_brushes;
        *selected_faces = entry.snapshot.selected_faces;
        *selected_patch_vertices = entry.snapshot.selected_patch_vertices;
        *selected_entity = entry.snapshot.selected_entity;
        *map_revision = map_revision.wrapping_add(1);
        Some(entry.label)
    }
}
