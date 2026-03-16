//! Small convenience methods on [`crate::map::Map`] used by editor frontends.

use crate::assets::resolve_editor_image_name;
use crate::map::{BrushContent, Map};
use crate::shader::ShaderDb;

impl Map {
    /// Count total brushes across all entities.
    pub fn count_brushes(&self) -> usize {
        self.entities.iter().map(|e| e.brushes.len()).sum()
    }

    /// Return the number of entities contained in the map.
    pub fn get_entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Collect a de-duplicated list of materials/shaders used by the map, using `qer_editorimage`
    /// when available.
    pub fn collect_used_materials(&self, shader_db: Option<&ShaderDb>) -> Vec<String> {
        let mut out = Vec::<String>::new();
        for ent in &self.entities {
            for b in &ent.brushes {
                match &b.content {
                    BrushContent::Convex(faces) => {
                        for f in faces {
                            out.push(resolve_editor_image_name(&f.texture, shader_db));
                        }
                    }
                    BrushContent::Patch(p) => {
                        out.push(resolve_editor_image_name(&p.shader, shader_db));
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Increment the map generation counter to signal that something changed.
    pub fn mark_map_dirty(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}
